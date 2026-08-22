use std::collections::HashMap;

use bevy::core_pipeline::oit::OrderIndependentTransparencySettings;
use bevy::core_pipeline::prepass::MotionVectorPrepass;
use bevy::core_pipeline::prepass::NormalPrepass;
use bevy::pbr::MeshPipelineKey;
use bevy::pbr::RenderMeshInstances;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::Local;
use bevy::prelude::Mesh3d;
use bevy::prelude::Msaa;
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
use bevy_render::sync_world::MainEntity;
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
use super::constants::NO_MESH_INSTANCE_FOUND_WARNING;
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
    views: Query<
        (
            Entity,
            &ExtractedView,
            &ExtractedCamera,
            &RenderVisibleEntities,
            &Msaa,
            Has<NormalPrepass>,
            Has<MotionVectorPrepass>,
            Has<OrderIndependentTransparencySettings>,
        ),
        With<OutlineCamera>,
    >,
    mut queued_entities: Local<HashMap<RetainedViewEntity, Vec<MainEntity>>>,
) {
    let draw_function_id = draw_functions.read().id::<DrawOutline>();

    for (
        _,
        view,
        camera,
        visible_entities,
        msaa,
        has_normal_prepass,
        has_motion_vector_prepass,
        has_oit,
    ) in views.iter()
    {
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

        let view_key = MeshPipelineKey::from_msaa_samples(msaa.samples())
            | MeshPipelineKey::DEPTH_PREPASS
            | include_pipeline_key(has_normal_prepass, MeshPipelineKey::NORMAL_PREPASS)
            | include_pipeline_key(
                has_motion_vector_prepass,
                MeshPipelineKey::MOTION_VECTOR_PREPASS,
            )
            | include_pipeline_key(!camera.hdr, MeshPipelineKey::TONEMAP_IN_SHADER)
            | include_pipeline_key(has_oit, MeshPipelineKey::OIT_ENABLED);

        for (render_entity, main_entity) in render_visible_entities.iter_visible() {
            let render_entity = *render_entity;
            let main_entity = *main_entity;
            if !extracted_outlines.by_main_entity.contains_key(&main_entity) {
                continue;
            }
            let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(main_entity)
            else {
                warn!(
                    target: LIMINAL_TRACING_TARGET,
                    "{NO_MESH_INSTANCE_FOUND_WARNING} {main_entity:?}"
                );
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

const fn include_pipeline_key(include: bool, key: MeshPipelineKey) -> MeshPipelineKey {
    if include {
        key
    } else {
        MeshPipelineKey::empty()
    }
}

/// Builds the view-level [`MeshPipelineKey`] for the hull outline pass: MSAA
/// sample count plus depth prepass, the normal prepass bit when present, the
/// in-shader tonemapping LUT bindings present whenever the camera is SDR, and
/// the OIT bindings when the camera uses order-independent transparency.
/// Matching the view's `mesh_view_bind_group` layout keeps the bind group
/// compatible at draw time; see `queue_outline`.
fn hull_view_key(
    msaa: Msaa,
    has_normal_prepass: bool,
    hdr: bool,
    has_oit: bool,
) -> MeshPipelineKey {
    let mut view_key =
        MeshPipelineKey::from_msaa_samples(msaa.samples()) | MeshPipelineKey::DEPTH_PREPASS;
    if has_normal_prepass {
        view_key |= MeshPipelineKey::NORMAL_PREPASS;
    }
    if !hdr {
        view_key |= MeshPipelineKey::TONEMAP_IN_SHADER;
    }
    if has_oit {
        view_key |= MeshPipelineKey::OIT_ENABLED;
    }
    view_key
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
            &Msaa,
            Has<NormalPrepass>,
            Has<OrderIndependentTransparencySettings>,
        ),
        With<OutlineCamera>,
    >,
    mut queued_entities: Local<HashMap<RetainedViewEntity, Vec<MainEntity>>>,
) {
    if !active.methods.has_hull() {
        return;
    }

    let draw_function_id = draw_functions.read().id::<DrawHull>();

    for (_, view, camera, visible_entities, msaa, has_normal_prepass, has_oit) in views.iter() {
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

        let view_key = hull_view_key(*msaa, has_normal_prepass, camera.hdr, has_oit);

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

            let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(main_entity)
            else {
                warn!(
                    target: LIMINAL_TRACING_TARGET,
                    "{NO_MESH_INSTANCE_FOUND_WARNING} {main_entity:?}"
                );
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
