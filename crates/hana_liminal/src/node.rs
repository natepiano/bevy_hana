use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::prelude::Entity;
use bevy::prelude::LinearRgba;
use bevy::prelude::Msaa;
use bevy::prelude::World;
use bevy::prelude::error;
use bevy::prelude::warn;
use bevy_render::camera::ExtractedCamera;
use bevy_render::render_asset::RenderAssets;
use bevy_render::render_phase::BinnedRenderPhase;
use bevy_render::render_phase::ViewBinnedRenderPhases;
use bevy_render::render_resource::BindGroupEntries;
use bevy_render::render_resource::LoadOp;
use bevy_render::render_resource::Operations;
use bevy_render::render_resource::PipelineCache;
use bevy_render::render_resource::RenderPassColorAttachment;
use bevy_render::render_resource::RenderPassDepthStencilAttachment;
use bevy_render::render_resource::RenderPassDescriptor;
use bevy_render::render_resource::StoreOp;
use bevy_render::render_resource::TextureView;
use bevy_render::render_resource::TextureViewDescriptor;
use bevy_render::renderer::RenderContext;
use bevy_render::renderer::ViewQuery;
use bevy_render::texture::GpuImage;
use bevy_render::view::ExtractedView;
use bevy_render::view::ViewDepthStencilTexture;
use bevy_render::view::ViewTarget;

use super::compose::ComposeOutputPipeline;
use super::compose::ComposeVariant;
use super::compose::SampleMode;
use super::constants::COMPOSE_BIND_GROUP_SLOT;
use super::constants::COMPOSE_OUTPUT_BIND_GROUP_LABEL;
use super::constants::FLOOD_INIT_RENDER_ERROR;
use super::constants::FULL_SCREEN_DRAW_INSTANCE_COUNT;
use super::constants::HULL_OUTLINE_PASS_LABEL;
use super::constants::HULL_OUTLINE_RENDER_ERROR;
use super::constants::JUMP_FLOOD_NO_SEED_CLEAR_COLOR;
use super::constants::NO_GLOBAL_DEPTH_TEXTURE_WARNING;
use super::constants::OUTLINE_DEPTH_FAR_PLANE_CLEAR;
use super::constants::OUTLINE_FLOOD_INIT_PASS_LABEL;
use super::constants::POST_PROCESS_PASS_LABEL;
use super::constants::TRIANGLE_VERTEX_COUNT;
use super::extract::ActiveOutlineModes;
use super::flood;
use super::flood::FloodSettings;
use super::flood::JumpFloodPass;
use super::flood::JumpFloodStep;
use super::hull_pipeline::DynamicRange;
use super::mask::HullOutlinePhase;
use super::mask::JumpFloodOutlinePhase;
use super::outline_depth::OutlineDepthTexture;
use super::texture::FloodTextures;
use crate::occlusion::TranslucentDepthPhase;

/// Populate the shared surface depth before overlays draw in the main pass.
/// Clear it even in views with no active outlines so retired surfaces cannot
/// occlude a later overlay.
pub(crate) fn outline_mask_pass(
    world: &World,
    view: ViewQuery<(
        Entity,
        &ExtractedView,
        &ExtractedCamera,
        &OutlineDepthTexture,
        Option<&FloodTextures>,
        &FloodSettings,
    )>,
    mut render_context: RenderContext,
) {
    let (view_entity, extracted_view, camera, depth, textures, settings) = view.into_inner();
    let images = world.resource::<RenderAssets<GpuImage>>();
    let (Some(image), Some(strokes)) = (images.get(&depth.surface), images.get(&depth.strokes))
    else {
        return;
    };
    if let Some(translucent) = images.get(&depth.translucent) {
        let mut pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("translucent surface depth"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view:        &translucent.texture_view,
                depth_ops:   Some(Operations {
                    load:  LoadOp::Clear(0.0),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        if let Some(viewport) = camera.viewport.as_ref() {
            pass.set_camera_viewport(viewport);
        }
        if let Some(phase) = world
            .resource::<ViewBinnedRenderPhases<TranslucentDepthPhase>>()
            .get(&extracted_view.retained_view_entity)
            && let Err(error) = phase.render(&mut pass, world, view_entity)
        {
            error!("translucent depth rendering failed: {error:?}");
        }
    }
    if let Some(flood_textures) = textures {
        let outline_phase = world
            .resource::<ViewBinnedRenderPhases<JumpFloodOutlinePhase>>()
            .get(&extracted_view.retained_view_entity);
        run_mask_init_pass(
            &mut render_context,
            MaskInitPassContext {
                flood_textures,
                outline_depth_view: &image.texture_view,
                camera,
                outline_phase,
                world,
                view_entity,
            },
        );
        if let Some(pass) = JumpFloodPass::new(world) {
            let mut textures = flood_textures.clone();
            for size in (0..flood::jump_flood_pass_count(settings.width)).rev() {
                textures.swap_ping_pong();
                pass.execute(
                    &mut render_context,
                    JumpFloodStep {
                        input: textures.input(),
                        output: textures.output(),
                        depth_view: &image.texture_view,
                        appearance_view: &textures.appearance.default_view,
                        size,
                    },
                );
            }
        }
    } else {
        let _pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some(OUTLINE_FLOOD_INIT_PASS_LABEL),
            color_attachments: &[Some(RenderPassColorAttachment {
                view:           &strokes.texture_view,
                resolve_target: None,
                ops:            Operations {
                    load:  LoadOp::Clear(JUMP_FLOOD_NO_SEED_CLEAR_COLOR.into()),
                    store: StoreOp::Store,
                },
                depth_slice:    None,
            })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view:        &image.texture_view,
                depth_ops:   Some(Operations {
                    load:  LoadOp::Clear(OUTLINE_DEPTH_FAR_PLANE_CLEAR),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
    }
}

struct MaskInitPassContext<'a> {
    flood_textures:     &'a FloodTextures,
    outline_depth_view: &'a TextureView,
    camera:             &'a ExtractedCamera,
    outline_phase:      Option<&'a BinnedRenderPhase<JumpFloodOutlinePhase>>,
    world:              &'a World,
    view_entity:        Entity,
}

struct HullPassContext<'a> {
    view_target:        &'a ViewTarget,
    view_depth_texture: &'a ViewDepthStencilTexture,
    camera:             &'a ExtractedCamera,
    phase:              &'a BinnedRenderPhase<HullOutlinePhase>,
    world:              &'a World,
    view_entity:        Entity,
}

struct JumpFloodCompositeContext<'a> {
    flood_textures:     &'a FloodTextures,
    outline_depth_view: &'a TextureView,
    global_depth:       &'a TextureView,
    view_target:        &'a ViewTarget,
    view_depth_texture: &'a ViewDepthStencilTexture,
    dynamic_range:      DynamicRange,
    msaa:               Msaa,
}

/// Renders the outline passes for one view. Runs in the `Core3d` schedule in
/// `Core3dSystems::EarlyPostProcess` (after the main pass, before bloom),
/// replacing the former `OutlineNode` render-graph node.
pub(crate) fn outline_pass(
    world: &World,
    view: ViewQuery<(
        Entity,
        &ExtractedView,
        &ExtractedCamera,
        &ViewTarget,
        &FloodTextures,
        &ViewPrepassTextures,
        &ViewDepthStencilTexture,
        &Msaa,
    )>,
    mut render_context: RenderContext,
) {
    let (
        view_entity,
        extracted_view,
        camera,
        view_target,
        flood_textures,
        prepass_textures,
        view_depth_texture,
        msaa,
    ) = view.into_inner();

    let Some(outline_phases) =
        world.get_resource::<ViewBinnedRenderPhases<JumpFloodOutlinePhase>>()
    else {
        return;
    };
    let outline_phase = outline_phases.get(&extracted_view.retained_view_entity);
    let hull_phase = world
        .get_resource::<ViewBinnedRenderPhases<HullOutlinePhase>>()
        .and_then(|phases| phases.get(&extracted_view.retained_view_entity));

    let has_jump_flood = outline_phase.is_some_and(|phase| !phase.is_empty());
    let has_hull = hull_phase.is_some_and(|phase| !phase.is_empty());
    if !has_jump_flood && !has_hull {
        return;
    }

    let hull_phase = hull_phase.filter(|phase| !phase.is_empty());

    let Some(jump_flood_pass) = JumpFloodPass::new(world) else {
        return;
    };
    let Some(global_depth) = prepass_textures.depth_only_view() else {
        warn!("{NO_GLOBAL_DEPTH_TEXTURE_WARNING}");
        return;
    };

    let outline_depth_view = flood_textures
        .outline_depth
        .create_view(&TextureViewDescriptor::default());

    if let Some(hull_phase) = hull_phase {
        run_hull_pass(
            &mut render_context,
            HullPassContext {
                view_target,
                view_depth_texture,
                camera,
                phase: hull_phase,
                world,
                view_entity,
            },
        );
    }

    run_jump_flood_composite(
        &mut render_context,
        world,
        &jump_flood_pass,
        JumpFloodCompositeContext {
            flood_textures,
            outline_depth_view: &outline_depth_view,
            global_depth,
            view_target,
            view_depth_texture,
            dynamic_range: DynamicRange::from(camera.hdr),
            msaa: *msaa,
        },
    );
}

fn run_mask_init_pass(
    render_context: &mut RenderContext<'_, '_>,
    mask_init_pass_context: MaskInitPassContext<'_>,
) {
    let MaskInitPassContext {
        flood_textures,
        outline_depth_view,
        camera,
        outline_phase,
        world,
        view_entity,
    } = mask_init_pass_context;
    let flood_color_attachment = RenderPassColorAttachment {
        view:           &flood_textures.output.default_view,
        resolve_target: None,
        ops:            Operations {
            load:  LoadOp::Clear(JUMP_FLOOD_NO_SEED_CLEAR_COLOR.into()),
            store: StoreOp::Store,
        },
        depth_slice:    None,
    };

    let appearance_color_attachment = RenderPassColorAttachment {
        view:           &flood_textures.appearance.default_view,
        resolve_target: None,
        ops:            Operations {
            load:  LoadOp::Clear(LinearRgba::NONE.into()),
            store: StoreOp::Store,
        },
        depth_slice:    None,
    };
    let owner_color_attachment = RenderPassColorAttachment {
        view:           &flood_textures.owner.default_view,
        resolve_target: None,
        ops:            Operations {
            load:  LoadOp::Clear(LinearRgba::NONE.into()),
            store: StoreOp::Store,
        },
        depth_slice:    None,
    };

    let mut init_pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
        label:                    Some(OUTLINE_FLOOD_INIT_PASS_LABEL),
        color_attachments:        &[
            Some(flood_color_attachment),
            Some(appearance_color_attachment),
            Some(owner_color_attachment),
        ],
        depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
            view:        outline_depth_view,
            depth_ops:   Some(Operations {
                load:  LoadOp::Clear(OUTLINE_DEPTH_FAR_PLANE_CLEAR),
                store: StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes:         None,
        occlusion_query_set:      None,
        multiview_mask:           None,
    });

    if let Some(viewport) = camera.viewport.as_ref() {
        init_pass.set_camera_viewport(viewport);
    }

    if let Some(outline_phase) = outline_phase
        && let Err(err) = outline_phase.render(&mut init_pass, world, view_entity)
    {
        error!("{FLOOD_INIT_RENDER_ERROR} {err:?}");
    }
}

fn run_hull_pass(
    render_context: &mut RenderContext<'_, '_>,
    hull_pass_context: HullPassContext<'_>,
) {
    let HullPassContext {
        view_target,
        view_depth_texture,
        camera,
        phase,
        world,
        view_entity,
    } = hull_pass_context;
    let mut hull_pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
        label:                    Some(HULL_OUTLINE_PASS_LABEL),
        color_attachments:        &[Some(view_target.get_color_attachment())],
        depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
            view:        view_depth_texture
                .attachment
                .depth_stencil_views()
                .attachment_view(),
            depth_ops:   Some(Operations {
                load:  LoadOp::Load,
                store: StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes:         None,
        occlusion_query_set:      None,
        multiview_mask:           None,
    });

    if let Some(viewport) = camera.viewport.as_ref() {
        hull_pass.set_camera_viewport(viewport);
    }

    if let Err(err) = phase.render(&mut hull_pass, world, view_entity) {
        error!("{HULL_OUTLINE_RENDER_ERROR} {err:?}");
    }
}

fn run_jump_flood_composite(
    render_context: &mut RenderContext<'_, '_>,
    world: &World,
    jump_flood_pass: &JumpFloodPass<'_>,
    jump_flood_composite_context: JumpFloodCompositeContext<'_>,
) {
    let JumpFloodCompositeContext {
        flood_textures,
        outline_depth_view,
        global_depth,
        view_target,
        view_depth_texture,
        dynamic_range,
        msaa,
    } = jump_flood_composite_context;
    let Some(active) = world.get_resource::<ActiveOutlineModes>() else {
        return;
    };
    if !active.methods.has_jump_flood() {
        return;
    }

    let Some(compose_pipeline) = world.get_resource::<ComposeOutputPipeline>() else {
        return;
    };

    let Some(main_depth_view) = view_depth_texture
        .attachment
        .depth_stencil_views()
        .depth_only_view()
    else {
        return;
    };
    let pipeline_cache = world.resource::<PipelineCache>();

    let sample_mode = SampleMode::from(msaa);
    let compose_variant = ComposeVariant::new(sample_mode, dynamic_range);
    let pipeline_id = compose_pipeline.pipeline_id(compose_variant);

    let Some(pipeline) = pipeline_cache.get_render_pipeline(pipeline_id) else {
        return;
    };

    let post_process = view_target.post_process_write();

    let bind_group = render_context.render_device().create_bind_group(
        COMPOSE_OUTPUT_BIND_GROUP_LABEL,
        &pipeline_cache.get_bind_group_layout(compose_pipeline.layout_for(compose_variant)),
        &BindGroupEntries::sequential((
            post_process.source,
            &jump_flood_pass.pipeline.sampler,
            &flood_textures.output.default_view,
            &flood_textures.appearance.default_view,
            global_depth,
            outline_depth_view,
            main_depth_view,
            &flood_textures.owner.default_view,
        )),
    );

    // Composite pass — write directly to `post_process.destination` rather than using
    // `view_target.get_color_attachment()` because the latter returns the multisampled
    // main texture when MSAA is enabled, which would require the pipeline to match
    // the MSAA sample count. The post-process destination is always single-sample.
    let mut render_pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
        label:                    Some(POST_PROCESS_PASS_LABEL),
        color_attachments:        &[Some(RenderPassColorAttachment {
            view:           post_process.destination,
            resolve_target: None,
            ops:            Operations::default(),
            depth_slice:    None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes:         None,
        occlusion_query_set:      None,
        multiview_mask:           None,
    });

    render_pass.set_render_pipeline(pipeline);
    render_pass.set_bind_group(COMPOSE_BIND_GROUP_SLOT, &bind_group, &[]);
    render_pass.draw(0..TRIANGLE_VERTEX_COUNT, 0..FULL_SCREEN_DRAW_INSTANCE_COUNT);
}
