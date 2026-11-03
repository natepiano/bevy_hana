use std::collections::HashMap;

use bevy::extract::sync_world::MainEntity;
use bevy::pbr::MeshPipelineKey;
use bevy::pbr::RenderMeshInstances;
use bevy::pbr::ViewKeyCache;
use bevy::prelude::Entity;
use bevy::prelude::Local;
use bevy::prelude::Mesh3d;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::With;
use bevy::prelude::warn;
use bevy_render::camera::ExtractedCamera;
use bevy_render::mesh::RenderMesh;
use bevy_render::mesh::allocator::MeshAllocator;
use bevy_render::render_asset::RenderAssets;
use bevy_render::render_phase::BinnedRenderPhase;
use bevy_render::render_phase::BinnedRenderPhaseType;
use bevy_render::render_phase::DrawFunctions;
use bevy_render::render_phase::ViewBinnedRenderPhases;
use bevy_render::render_resource::PipelineCache;
use bevy_render::render_resource::SpecializedMeshPipelines;
use bevy_render::view::ExtractedView;
use bevy_render::view::RenderVisibleEntities;
use bevy_render::view::RetainedViewEntity;

use super::DrawHull;
use super::DrawOutline;
use super::camera::OutlineCamera;
use super::constants::ATTRIBUTE_OUTLINE_NORMAL;
use super::constants::FAILED_TO_SPECIALIZE_HULL_MESH_PIPELINE_WARNING;
use super::constants::FAILED_TO_SPECIALIZE_MESH_PIPELINE_WARNING;
use super::constants::LIMINAL_TRACING_TARGET;
use super::constants::NO_MESH_FOUND_WARNING;
use super::extract::ActiveOutlineModes;
use super::extract::ExtractedOutlineUniforms;
use super::hull_pipeline::HullPipeline;
use super::hull_pipeline::HullPipelineKey;
use super::hull_pipeline::OutlineNormalPresence;
use super::mask::HullOutlinePhase;
use super::mask::JumpFloodOutlinePhase;
use super::mask::OutlineBatchSetKey;
use super::mask::OutlineBinKey;
use super::mask_pipeline::MeshMaskPipeline;
use super::outline::OutlineMethod;
use super::render::DrawTranslucentDepth;
use crate::occlusion::TranslucentDepthPhase;
use crate::occlusion::TranslucentDepthPipeline;
use crate::occlusion::TranslucentOccluders;

pub(crate) fn queue_outline(
    extracted_outlines: Res<ExtractedOutlineUniforms>,
    draw_functions: Res<DrawFunctions<JumpFloodOutlinePhase>>,
    mut outline_phases: ResMut<ViewBinnedRenderPhases<JumpFloodOutlinePhase>>,
    mesh_mask_pipeline: Res<MeshMaskPipeline>,
    mut mesh_mask_pipelines: ResMut<SpecializedMeshPipelines<MeshMaskPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    mesh_allocator: Res<MeshAllocator>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    render_mesh_instances: Res<RenderMeshInstances>,
    views: Query<(Entity, &ExtractedView, &RenderVisibleEntities), With<OutlineCamera>>,
    view_key_cache: Res<ViewKeyCache>,
    mut queued_entities: Local<HashMap<RetainedViewEntity, Vec<MainEntity>>>,
) {
    let draw_function_id = draw_functions.read().id::<DrawOutline>();

    for (_, view, visible_entities) in views.iter() {
        let Some(outline_phase) = outline_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };

        let previously_queued = clear_previous_outline_bins(
            outline_phase,
            &mut queued_entities,
            view.retained_view_entity,
        );

        let Some(render_visible_entities) = visible_entities.get::<Mesh3d>() else {
            continue;
        };

        let Some(&view_key) = view_key_cache.get(&view.retained_view_entity) else {
            continue;
        };

        for (render_entity, main_entity) in render_visible_entities.iter_visible() {
            let render_entity = *render_entity;
            let main_entity = *main_entity;
            if !extracted_outlines.by_main_entity.contains_key(&main_entity) {
                continue;
            }
            // Bevy holds back a new mesh's instance until its material is
            // prepared, and its own queues skip it as pending. This queue
            // re-bins every visible entity each frame, so the outline draws
            // the frame the instance exists.
            let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(main_entity)
            else {
                continue;
            };

            let Some(mesh) = render_meshes.get(mesh_instance.mesh_asset_id()) else {
                warn!(
                    target: LIMINAL_TRACING_TARGET,
                    "{NO_MESH_FOUND_WARNING} {main_entity:?}"
                );
                continue;
            };

            let mut mesh_pipeline_key = view_key;
            mesh_pipeline_key |= MeshPipelineKey::from_primitive_topology_and_strip_index(
                mesh.primitive_topology(),
                mesh.index_format(),
            ) | MeshPipelineKey::from_bits_retain(mesh.key_bits.bits());

            let Ok(pipeline_id) = mesh_mask_pipelines.specialize(
                &pipeline_cache,
                &mesh_mask_pipeline,
                mesh_pipeline_key,
                &mesh.layout,
            ) else {
                warn!(
                    target: LIMINAL_TRACING_TARGET,
                    "{FAILED_TO_SPECIALIZE_MESH_PIPELINE_WARNING}"
                );
                continue;
            };

            outline_phase.add(
                OutlineBatchSetKey {
                    cached_render_pipeline_id: pipeline_id,
                    draw_function_id,
                    mesh_slabs: mesh_allocator
                        .mesh_slabs(&mesh_instance.mesh_asset_id())
                        .unwrap_or_default(),
                },
                OutlineBinKey {
                    asset_id: mesh_instance.mesh_asset_id().untyped(),
                    main_entity,
                },
                (render_entity, main_entity),
                mesh_instance.current_uniform_index,
                // Force batchable/unbatchable (never multidrawable): the outline uniform
                // buffer in `render.rs` is built by mirroring the public batchable and
                // unbatchable bins, and 0.19 makes multidrawable bin contents private.
                if mesh_instance.should_batch() {
                    BinnedRenderPhaseType::BatchableMesh
                } else {
                    BinnedRenderPhaseType::UnbatchableMesh
                },
            );
            previously_queued.push(main_entity);
        }
    }
}

fn clear_previous_outline_bins<'a>(
    outline_phase: &mut BinnedRenderPhase<JumpFloodOutlinePhase>,
    queued_entities: &'a mut HashMap<RetainedViewEntity, Vec<MainEntity>>,
    retained_view_entity: RetainedViewEntity,
) -> &'a mut Vec<MainEntity> {
    let previously_queued = queued_entities.entry(retained_view_entity).or_default();
    for &main_entity in previously_queued.iter() {
        outline_phase.remove(main_entity);
    }
    previously_queued.clear();
    previously_queued
}

pub(crate) fn queue_hull_outline(
    active: Res<ActiveOutlineModes>,
    extracted_outlines: Res<ExtractedOutlineUniforms>,
    draw_functions: Res<DrawFunctions<HullOutlinePhase>>,
    mut outline_phases: ResMut<ViewBinnedRenderPhases<HullOutlinePhase>>,
    hull_pipeline: Res<HullPipeline>,
    mut hull_pipelines: ResMut<SpecializedMeshPipelines<HullPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    mesh_allocator: Res<MeshAllocator>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    render_mesh_instances: Res<RenderMeshInstances>,
    views: Query<
        (
            Entity,
            &ExtractedView,
            &ExtractedCamera,
            &RenderVisibleEntities,
        ),
        With<OutlineCamera>,
    >,
    view_key_cache: Res<ViewKeyCache>,
    mut queued_entities: Local<HashMap<RetainedViewEntity, Vec<MainEntity>>>,
) {
    if !active.methods.has_hull() {
        return;
    }

    let draw_function_id = draw_functions.read().id::<DrawHull>();

    for (_, view, camera, visible_entities) in views.iter() {
        let Some(outline_phase) = outline_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };

        // Binned phases are retained across frames in 0.19, so explicitly remove the
        // entities binned last frame before re-binning this frame's visible set.
        let previously_queued = queued_entities
            .entry(view.retained_view_entity)
            .or_default();
        for &main_entity in previously_queued.iter() {
            outline_phase.remove(main_entity);
        }
        previously_queued.clear();

        let Some(render_visible_entities) = visible_entities.get::<Mesh3d>() else {
            continue;
        };

        let Some(&view_key) = view_key_cache.get(&view.retained_view_entity) else {
            continue;
        };

        for (render_entity, main_entity) in render_visible_entities.iter_visible() {
            let render_entity = *render_entity;
            let main_entity = *main_entity;
            let Some(outline) = extracted_outlines.by_main_entity.get(&main_entity) else {
                continue;
            };
            if !matches!(
                outline.outline_method,
                OutlineMethod::WorldHull | OutlineMethod::ScreenHull
            ) {
                continue;
            }

            // Bevy holds back a new mesh's instance until its material is
            // prepared, and its own queues skip it as pending. This queue
            // re-bins every visible entity each frame, so the outline draws
            // the frame the instance exists.
            let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(main_entity)
            else {
                continue;
            };

            let Some(mesh) = render_meshes.get(mesh_instance.mesh_asset_id()) else {
                warn!(
                    target: LIMINAL_TRACING_TARGET,
                    "{NO_MESH_FOUND_WARNING} {main_entity:?}"
                );
                continue;
            };

            let mut mesh_pipeline_key = view_key;
            mesh_pipeline_key |= MeshPipelineKey::from_primitive_topology_and_strip_index(
                mesh.primitive_topology(),
                mesh.index_format(),
            ) | MeshPipelineKey::from_bits_retain(mesh.key_bits.bits());

            let Ok(pipeline_id) = hull_pipelines.specialize(
                &pipeline_cache,
                &hull_pipeline,
                HullPipelineKey {
                    mesh_pipeline_key,
                    dynamic_range: camera.hdr.into(),
                    outline_normal_presence: if mesh.layout.0.contains(ATTRIBUTE_OUTLINE_NORMAL) {
                        OutlineNormalPresence::Present
                    } else {
                        OutlineNormalPresence::Absent
                    },
                },
                &mesh.layout,
            ) else {
                warn!(
                    target: LIMINAL_TRACING_TARGET,
                    "{FAILED_TO_SPECIALIZE_HULL_MESH_PIPELINE_WARNING}"
                );
                continue;
            };

            outline_phase.add(
                OutlineBatchSetKey {
                    cached_render_pipeline_id: pipeline_id,
                    draw_function_id,
                    mesh_slabs: mesh_allocator
                        .mesh_slabs(&mesh_instance.mesh_asset_id())
                        .unwrap_or_default(),
                },
                OutlineBinKey {
                    asset_id: mesh_instance.mesh_asset_id().untyped(),
                    main_entity,
                },
                (render_entity, main_entity),
                mesh_instance.current_uniform_index,
                // Force batchable/unbatchable (never multidrawable): the outline uniform
                // buffer in `render.rs` is built by mirroring the public batchable and
                // unbatchable bins, and 0.19 makes multidrawable bin contents private.
                if mesh_instance.should_batch() {
                    BinnedRenderPhaseType::BatchableMesh
                } else {
                    BinnedRenderPhaseType::UnbatchableMesh
                },
            );
            previously_queued.push(main_entity);
        }
    }
}

pub(crate) fn queue_translucent_depth(
    occluders: Res<TranslucentOccluders>,
    draw_functions: Res<DrawFunctions<TranslucentDepthPhase>>,
    mut translucent_phases: ResMut<ViewBinnedRenderPhases<TranslucentDepthPhase>>,
    translucent_pipeline: Res<TranslucentDepthPipeline>,
    mut translucent_pipelines: ResMut<SpecializedMeshPipelines<TranslucentDepthPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    mesh_allocator: Res<MeshAllocator>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    render_mesh_instances: Res<RenderMeshInstances>,
    views: Query<(Entity, &ExtractedView, &RenderVisibleEntities), With<OutlineCamera>>,
    view_key_cache: Res<ViewKeyCache>,
    mut queued_entities: Local<HashMap<RetainedViewEntity, Vec<MainEntity>>>,
) {
    let draw_function_id = draw_functions.read().id::<DrawTranslucentDepth>();

    for (_, view, visible_entities) in views.iter() {
        let Some(translucent_phase) = translucent_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };

        let previously_queued = clear_previous_translucent_bins(
            translucent_phase,
            &mut queued_entities,
            view.retained_view_entity,
        );

        let Some(render_visible_entities) = visible_entities.get::<Mesh3d>() else {
            continue;
        };

        let Some(&view_key) = view_key_cache.get(&view.retained_view_entity) else {
            continue;
        };

        for (render_entity, main_entity) in render_visible_entities.iter_visible() {
            let render_entity = *render_entity;
            let main_entity = *main_entity;
            if !occluders.0.contains(&main_entity) {
                continue;
            }
            // Bevy holds back a new mesh's instance until its material is
            // prepared, and its own queues skip it as pending. This queue
            // re-bins every visible entity each frame, so depth draws
            // the frame the instance exists.
            let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(main_entity)
            else {
                continue;
            };

            let Some(mesh) = render_meshes.get(mesh_instance.mesh_asset_id()) else {
                warn!(
                    target: LIMINAL_TRACING_TARGET,
                    "{NO_MESH_FOUND_WARNING} {main_entity:?}"
                );
                continue;
            };

            let mut mesh_pipeline_key = view_key;
            mesh_pipeline_key |= MeshPipelineKey::from_primitive_topology_and_strip_index(
                mesh.primitive_topology(),
                mesh.index_format(),
            ) | MeshPipelineKey::from_bits_retain(mesh.key_bits.bits());

            let Ok(pipeline_id) = translucent_pipelines.specialize(
                &pipeline_cache,
                &translucent_pipeline,
                mesh_pipeline_key,
                &mesh.layout,
            ) else {
                warn!(
                    target: LIMINAL_TRACING_TARGET,
                    "{FAILED_TO_SPECIALIZE_MESH_PIPELINE_WARNING}"
                );
                continue;
            };

            translucent_phase.add(
                OutlineBatchSetKey {
                    cached_render_pipeline_id: pipeline_id,
                    draw_function_id,
                    mesh_slabs: mesh_allocator
                        .mesh_slabs(&mesh_instance.mesh_asset_id())
                        .unwrap_or_default(),
                },
                OutlineBinKey {
                    asset_id: mesh_instance.mesh_asset_id().untyped(),
                    main_entity,
                },
                (render_entity, main_entity),
                mesh_instance.current_uniform_index,
                // Match this phase's PreprocessingOnly mode (no multidraw bins).
                if mesh_instance.should_batch() {
                    BinnedRenderPhaseType::BatchableMesh
                } else {
                    BinnedRenderPhaseType::UnbatchableMesh
                },
            );
            previously_queued.push(main_entity);
        }
    }
}

fn clear_previous_translucent_bins<'a>(
    translucent_phase: &mut BinnedRenderPhase<TranslucentDepthPhase>,
    queued_entities: &'a mut HashMap<RetainedViewEntity, Vec<MainEntity>>,
    retained_view_entity: RetainedViewEntity,
) -> &'a mut Vec<MainEntity> {
    let previously_queued = queued_entities.entry(retained_view_entity).or_default();
    for &main_entity in previously_queued.iter() {
        translucent_phase.remove(main_entity);
    }
    previously_queued.clear();
    previously_queued
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    use bevy::asset::RenderAssetUsages;
    use bevy::camera::RenderTarget;
    use bevy::log::BoxedLayer;
    use bevy::log::LogPlugin;
    use bevy::log::tracing;
    use bevy::log::tracing::Event;
    use bevy::log::tracing_subscriber::Layer;
    use bevy::log::tracing_subscriber::layer::Context;
    use bevy::prelude::*;
    use bevy::render::RenderApp;
    use bevy::render::RenderPlugin;
    use bevy::render::pipelined_rendering::PipelinedRenderingPlugin;
    use bevy::render::render_resource::Extent3d;
    use bevy::render::render_resource::TextureDimension;
    use bevy::render::render_resource::TextureFormat;
    use bevy::window::ExitCondition;
    use bevy::winit::WinitPlugin;

    use super::*;
    use crate::LiminalPlugin;
    use crate::Outline;

    /// Counts `hana_liminal` warnings on every thread. `LogPlugin` makes this
    /// layer the process's global subscriber, and nextest gives each test a
    /// process of its own.
    #[derive(Clone, Default, Resource)]
    struct LiminalWarnings(Arc<AtomicUsize>);

    impl<S: tracing::Subscriber> Layer<S> for LiminalWarnings {
        fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
            let metadata = event.metadata();
            if *metadata.level() == tracing::Level::WARN
                && metadata.target() == LIMINAL_TRACING_TARGET
            {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    fn outline_binned(app: &App, entity: MainEntity) -> bool {
        app.get_sub_app(RenderApp)
            .expect("render app")
            .world()
            .resource::<ViewBinnedRenderPhases<JumpFloodOutlinePhase>>()
            .values()
            .any(|phase| {
                phase
                    .batchable_meshes
                    .keys()
                    .chain(phase.unbatchable_meshes.keys())
                    .any(|(_, bin)| bin.main_entity == entity)
            })
    }

    /// Bevy holds back a new mesh's instance until its material is prepared,
    /// as when a texture is still on its way, and draws it the frame after.
    #[test]
    fn an_outlined_mesh_waiting_on_its_material_logs_no_warning_and_outlines_once_ready() {
        let mut app = App::new();
        app.add_plugins((
            DefaultPlugins
                .set(LogPlugin {
                    custom_layer: |app| {
                        let warnings = LiminalWarnings::default();
                        app.insert_resource(warnings.clone());
                        Some(Box::new(warnings) as BoxedLayer)
                    },
                    ..default()
                })
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
            LiminalPlugin,
        ));
        app.finish();
        app.cleanup();
        let warnings = app.world().resource::<LiminalWarnings>().0.clone();
        warn!(target: LIMINAL_TRACING_TARGET, "captured from the test thread");
        assert_eq!(
            warnings.load(Ordering::SeqCst),
            1,
            "the log layer counts warnings"
        );

        let world = app.world_mut();
        let target = world
            .resource_mut::<Assets<Image>>()
            .add(Image::new_target_texture(
                64,
                64,
                TextureFormat::Rgba8UnormSrgb,
                None,
            ));
        world.spawn((
            Camera3d::default(),
            RenderTarget::Image(target.into()),
            OutlineCamera,
            Msaa::Off,
            Transform::from_xyz(0.0, 0.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        let texture = world.resource::<Assets<Image>>().reserve_handle();
        let mesh = world.resource_mut::<Assets<Mesh>>().add(Cuboid::default());
        let material = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color_texture: Some(texture.clone()),
                ..default()
            });
        let cube = MainEntity::from(
            world
                .spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    Outline::jump_flood(4.0).build(),
                ))
                .id(),
        );

        for _ in 0..3 {
            app.update();
        }
        assert!(
            !outline_binned(&app, cube),
            "the cube's material waits on its texture, so Bevy holds back its mesh instance"
        );

        app.world_mut()
            .resource_mut::<Assets<Image>>()
            .insert(
                &texture,
                Image::new_fill(
                    Extent3d::default(),
                    TextureDimension::D2,
                    &[u8::MAX; 4],
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::default(),
                ),
            )
            .expect("the reserved texture");
        // The loop ends on the frame the outline queues; nextest's slow-timeout
        // ends a hang.
        while !outline_binned(&app, cube) {
            app.update();
        }
        assert_eq!(
            warnings.load(Ordering::SeqCst),
            1,
            "a mesh waiting on its material is pending, not a fault"
        );
    }
}
