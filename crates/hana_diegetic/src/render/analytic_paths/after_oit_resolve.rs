//! Draws additive and multiplicative path text over resolved OIT panels.

use std::any::TypeId;
use std::ops::Range;

use bevy::camera::MainPassResolutionOverride;
use bevy::camera::Viewport;
use bevy::core_pipeline::core_3d::Transparent3d;
use bevy::core_pipeline::oit::OrderIndependentTransparencySettings;
use bevy::core_pipeline::oit::resolve::OitResolvePipelineId;
use bevy::core_pipeline::oit::resolve::node::oit_resolve;
use bevy::core_pipeline::schedule::Core3d;
use bevy::core_pipeline::schedule::Core3dSystems;
use bevy::ecs::entity::EntityHash;
use bevy::ecs::world::World;
use bevy::extract::sync_world::MainEntity;
use bevy::extract::sync_world::MainEntityHashSet;
use bevy::math::FloatOrd;
use bevy::pbr::DrawMaterial;
use bevy::pbr::MeshPipeline;
use bevy::pbr::MeshPipelineKey;
use bevy::pbr::PendingMeshMaterialQueues;
use bevy::pbr::PreparedMaterial;
use bevy::pbr::RenderMaterialInstances;
use bevy::pbr::ViewKeyCache;
use bevy::prelude::AlphaMode;
use bevy::prelude::App;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Local;
use bevy::prelude::Mesh3d;
use bevy::prelude::Plugin;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::render::Render;
use bevy::render::RenderApp;
use bevy::render::RenderDebugFlags;
use bevy::render::RenderSystems;
use bevy::render::camera::DirtySpecializations;
use bevy::render::camera::ExtractedCamera;
use bevy::render::erased_render_asset::ErasedRenderAssets;
use bevy::render::render_phase::AddRenderCommand;
use bevy::render::render_phase::CachedRenderPipelinePhaseItem;
use bevy::render::render_phase::DrawFunctionId;
use bevy::render::render_phase::DrawFunctions;
use bevy::render::render_phase::PhaseItem;
use bevy::render::render_phase::PhaseItemExtraIndex;
use bevy::render::render_phase::SortedPhaseItem;
use bevy::render::render_phase::SortedRenderPhase;
use bevy::render::render_phase::SortedRenderPhasePlugin;
use bevy::render::render_phase::ViewSortedRenderPhases;
use bevy::render::render_phase::sort_phase_system;
use bevy::render::render_resource::AsBindGroup;
use bevy::render::render_resource::CachedRenderPipelineId;
use bevy::render::render_resource::PipelineCache;
use bevy::render::render_resource::RenderPassDescriptor;
use bevy::render::render_resource::StoreOp;
use bevy::render::renderer::RenderContext;
use bevy::render::renderer::ViewQuery;
use bevy::render::view::ExtractedView;
use bevy::render::view::RenderVisibleEntities;
use bevy::render::view::ViewDepthStencilTexture;
use bevy::render::view::ViewTarget;
use indexmap::IndexMap;

use super::material::PathExtendedMaterial;
use super::material::PathExtensionKey;
use crate::constants::DIAGNOSTIC_LOG_TARGET;

/// A path draw whose blend operation needs the resolved OIT color target.
pub struct PostOitResolveText3d {
    source:        Transparent3d,
    draw_function: DrawFunctionId,
}

impl PostOitResolveText3d {
    pub(in crate::render) const fn from_transparent(
        source: Transparent3d,
        draw_function: DrawFunctionId,
    ) -> Self {
        Self {
            source,
            draw_function,
        }
    }
}

impl PhaseItem for PostOitResolveText3d {
    fn entity(&self) -> Entity { self.source.entity.0 }
    fn main_entity(&self) -> MainEntity { self.source.entity.1 }
    fn draw_function(&self) -> DrawFunctionId { self.draw_function }
    fn batch_range(&self) -> &Range<u32> { &self.source.batch_range }
    fn batch_range_mut(&mut self) -> &mut Range<u32> { &mut self.source.batch_range }
    fn extra_index(&self) -> PhaseItemExtraIndex { self.source.extra_index.clone() }
    fn batch_range_and_extra_index_mut(&mut self) -> (&mut Range<u32>, &mut PhaseItemExtraIndex) {
        (&mut self.source.batch_range, &mut self.source.extra_index)
    }
}

impl SortedPhaseItem for PostOitResolveText3d {
    type SortKey = FloatOrd;

    fn sort_key(&self) -> Self::SortKey { FloatOrd(self.source.distance) }

    fn recalculate_sort_keys(
        items: &mut IndexMap<(Entity, MainEntity), Self, EntityHash>,
        view: &ExtractedView,
    ) {
        let rangefinder = view.rangefinder3d();
        for item in items.values_mut() {
            item.source.distance = item.source.sorting_info.sort_distance(&rangefinder);
        }
    }

    fn indexed(&self) -> bool { self.source.indexed }
}

impl CachedRenderPipelinePhaseItem for PostOitResolveText3d {
    fn cached_pipeline(&self) -> CachedRenderPipelineId { self.source.pipeline }
}

pub(super) struct PostOitResolveTextPlugin;

impl Plugin for PostOitResolveTextPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(
            SortedRenderPhasePlugin::<PostOitResolveText3d, MeshPipeline>::new(
                RenderDebugFlags::default(),
            ),
        );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<DrawFunctions<PostOitResolveText3d>>()
            .add_render_command::<PostOitResolveText3d, DrawMaterial>()
            .add_systems(
                Render,
                (
                    move_post_resolve_text
                        .after(RenderSystems::QueueMeshes)
                        .before(RenderSystems::PhaseSort),
                    sort_phase_system::<PostOitResolveText3d>.in_set(RenderSystems::PhaseSort),
                ),
            )
            .add_systems(
                Core3d,
                draw_post_resolve_text
                    .after(oit_resolve)
                    .in_set(Core3dSystems::MainPass),
            );
    }
}

fn move_post_resolve_text(
    views: Query<(
        &ExtractedView,
        &RenderVisibleEntities,
        Has<OrderIndependentTransparencySettings>,
    )>,
    view_keys: Res<ViewKeyCache>,
    dirty_specializations: Res<DirtySpecializations>,
    pending_material_queues: Res<PendingMeshMaterialQueues>,
    material_instances: Res<RenderMaterialInstances>,
    materials: Res<ErasedRenderAssets<PreparedMaterial>>,
    draw_functions: Res<DrawFunctions<PostOitResolveText3d>>,
    mut transparent_phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    mut post_phases: ResMut<ViewSortedRenderPhases<PostOitResolveText3d>>,
    mut queued_meshes: Local<MainEntityHashSet>,
) {
    let draw_function = draw_functions.read().id::<DrawMaterial>();
    for (view, visible, has_oit) in &views {
        post_phases.prepare_for_new_frame(view.retained_view_entity);
        let Some(post_phase) = post_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        // Both phases are retained. Bevy only queues changed transparent items,
        // so unchanged moved items must stay here until its dequeue signal.
        let oit_enabled = has_oit
            && view_keys
                .get(&view.retained_view_entity)
                .is_some_and(|key| key.contains(MeshPipelineKey::OIT_ENABLED));
        if !oit_enabled {
            post_phase.clear();
            continue;
        }
        let Some(visible_meshes) = visible.get::<Mesh3d>() else {
            post_phase.clear();
            continue;
        };
        remove_dequeued_items(
            post_phase,
            dirty_specializations.iter_to_dequeue(view.retained_view_entity, visible_meshes),
        );
        let Some(transparent_phase) = transparent_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Some(pending_queue) = pending_material_queues.get(&view.retained_view_entity) else {
            continue;
        };
        let candidates = dirty_specializations
            .iter_to_queue(
                view.retained_view_entity,
                visible_meshes,
                &pending_queue.prev_frame,
                &mut queued_meshes,
            )
            .map(|(_, main_entity)| *main_entity);
        move_items_for_view(
            transparent_phase,
            post_phase,
            draw_function,
            candidates,
            |item| {
                let Some(material_instance) = material_instances.instances.get(&item.entity.1)
                else {
                    return false;
                };
                if material_instance.asset_id.type_id() != TypeId::of::<PathExtendedMaterial>() {
                    return false;
                }
                materials
                    .get(material_instance.asset_id)
                    .is_some_and(|material| {
                        let key: <PathExtendedMaterial as AsBindGroup>::Data =
                            material.properties.material_key.to_key();
                        draws_after_oit_resolve(material.properties.alpha_mode, key.extension)
                    })
            },
        );
    }
    prune_inactive_view_phases(&transparent_phases, &mut post_phases);
}

fn prune_inactive_view_phases(
    transparent_phases: &ViewSortedRenderPhases<Transparent3d>,
    post_phases: &mut ViewSortedRenderPhases<PostOitResolveText3d>,
) {
    post_phases.retain(|view_entity, _| transparent_phases.contains_key(view_entity));
}

fn remove_dequeued_items<'a>(
    post_phase: &mut SortedRenderPhase<PostOitResolveText3d>,
    dequeued: impl IntoIterator<Item = &'a MainEntity>,
) {
    for &main_entity in dequeued {
        post_phase.remove(Entity::PLACEHOLDER, main_entity);
    }
}

const fn draws_after_oit_resolve(alpha_mode: AlphaMode, key: PathExtensionKey) -> bool {
    key.is_panel_text && matches!(alpha_mode, AlphaMode::Add | AlphaMode::Multiply)
}

fn move_items_for_view(
    transparent_phase: &mut SortedRenderPhase<Transparent3d>,
    post_phase: &mut SortedRenderPhase<PostOitResolveText3d>,
    draw_function: DrawFunctionId,
    candidates: impl IntoIterator<Item = MainEntity>,
    should_move: impl Fn(&Transparent3d) -> bool,
) {
    for main_entity in candidates {
        let key = (Entity::PLACEHOLDER, main_entity);
        if transparent_phase.items.get(&key).is_some_and(&should_move)
            && let Some(source) = transparent_phase.items.swap_remove(&key)
        {
            post_phase.add_retained(PostOitResolveText3d::from_transparent(
                source,
                draw_function,
            ));
        }
    }
}

fn draw_post_resolve_text(
    world: &World,
    view: ViewQuery<(
        &ExtractedCamera,
        &ExtractedView,
        &ViewTarget,
        &ViewDepthStencilTexture,
        &OitResolvePipelineId,
        Option<&MainPassResolutionOverride>,
        Has<OrderIndependentTransparencySettings>,
    )>,
    post_phases: Res<ViewSortedRenderPhases<PostOitResolveText3d>>,
    pipeline_cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let view_entity = view.entity();
    let (camera, extracted_view, target, depth, resolve_pipeline, resolution_override, has_oit) =
        view.into_inner();
    if !has_oit
        || pipeline_cache
            .get_render_pipeline(resolve_pipeline.0)
            .is_none()
    {
        return;
    }
    let Some(phase) = post_phases.get(&extracted_view.retained_view_entity) else {
        return;
    };
    if phase.items.is_empty() {
        return;
    }
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label:                    Some("post_oit_resolve_text"),
        color_attachments:        &[Some(target.get_color_attachment())],
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        timestamp_writes:         None,
        occlusion_query_set:      None,
        multiview_mask:           None,
    });
    if let Some(viewport) =
        Viewport::from_viewport_and_override(camera.viewport.as_ref(), resolution_override)
    {
        pass.set_camera_viewport(&viewport);
    }
    if let Err(error) = phase.render(&mut pass, world, view_entity) {
        bevy::log::error!(target: DIAGNOSTIC_LOG_TARGET, "Error rendering post-OIT text phase: {error:?}");
    }
}

#[cfg(test)]
mod tests {
    use bevy::core_pipeline::core_3d::TransparentSortingInfo3d;
    use bevy::ecs::world::World;
    use bevy::prelude::Handle;
    use bevy::render::view::RetainedViewEntity;

    use super::*;
    use crate::render::AntiAlias;
    use crate::render::analytic_paths::material::PathExtension;
    use crate::render::analytic_paths::material::PathMaterialBuffers;
    use crate::render::analytic_paths::material::RenderMode;

    fn buffers() -> PathMaterialBuffers {
        PathMaterialBuffers {
            curves:       Handle::default(),
            bands:        Handle::default(),
            path_records: Handle::default(),
            instances:    Handle::default(),
            run_records:  Handle::default(),
        }
    }

    #[test]
    fn only_add_and_multiply_panel_text_moves_after_oit_resolve() {
        let text =
            super::super::text_vertex_pull(AlphaMode::Add, 0.0, AntiAlias::default(), buffers());
        let shape = PathExtension::vertex_pull(
            RenderMode::Text,
            AlphaMode::Add,
            0.0,
            AntiAlias::default(),
            buffers(),
        );
        let text_key = PathExtensionKey::from(&text);
        let shape_key = PathExtensionKey::from(&shape);
        assert!(draws_after_oit_resolve(AlphaMode::Add, text_key));
        assert!(draws_after_oit_resolve(AlphaMode::Multiply, text_key));
        assert!(!draws_after_oit_resolve(AlphaMode::Blend, text_key));
        assert!(!draws_after_oit_resolve(AlphaMode::Add, shape_key));
        assert!(!draws_after_oit_resolve(AlphaMode::Multiply, shape_key));
    }

    fn item(world: &mut World, distance: f32) -> Transparent3d {
        let render = Entity::PLACEHOLDER;
        let main = MainEntity::from(world.spawn_empty().id());
        Transparent3d {
            sorting_info: TransparentSortingInfo3d::AlwaysOnTop,
            distance,
            pipeline: CachedRenderPipelineId::INVALID,
            entity: (render, main),
            draw_function: DrawFunctionId(1),
            batch_range: 0..1,
            extra_index: PhaseItemExtraIndex::None,
            indexed: false,
        }
    }

    #[test]
    fn newly_queued_add_and_multiply_text_moves_out_of_transparent_phase() {
        let mut world = World::new();
        let mut transparent = SortedRenderPhase::<Transparent3d>::default();
        let add = item(&mut world, 3.0);
        let multiply = item(&mut world, 2.0);
        let blend = item(&mut world, 1.0);
        let add_key = add.entity;
        let multiply_key = multiply.entity;
        let blend_key = blend.entity;
        transparent.add_retained(add);
        transparent.add_retained(multiply);
        transparent.add_retained(blend);
        let mut post = SortedRenderPhase::<PostOitResolveText3d>::default();
        let is_add_or_multiply =
            |item: &Transparent3d| item.entity == add_key || item.entity == multiply_key;
        move_items_for_view(
            &mut transparent,
            &mut post,
            DrawFunctionId(2),
            [add_key.1, multiply_key.1, blend_key.1],
            is_add_or_multiply,
        );
        assert_eq!(transparent.items.len(), 1);
        assert!(transparent.items.contains_key(&blend_key));
        assert_eq!(post.items.len(), 2);
        assert_eq!(post.items[&add_key].source.distance, 3.0);
        assert_eq!(post.items[&multiply_key].draw_function(), DrawFunctionId(2));
    }

    #[test]
    fn retained_transparent_items_outside_this_frames_queue_are_not_inspected() {
        let mut world = World::new();
        let mut transparent = SortedRenderPhase::<Transparent3d>::default();
        let queued = item(&mut world, 2.0);
        let retained = item(&mut world, 1.0);
        let queued_key = queued.entity;
        let retained_key = retained.entity;
        transparent.add_retained(queued);
        transparent.add_retained(retained);
        let mut post = SortedRenderPhase::<PostOitResolveText3d>::default();
        move_items_for_view(
            &mut transparent,
            &mut post,
            DrawFunctionId(2),
            [queued_key.1],
            |_| true,
        );
        assert!(post.items.contains_key(&queued_key));
        assert!(transparent.items.contains_key(&retained_key));
    }

    #[test]
    fn post_resolve_phases_follow_active_transparent_views() {
        let active = RetainedViewEntity::new(MainEntity::from(Entity::from_bits(1)), None, 0);
        let inactive = RetainedViewEntity::new(MainEntity::from(Entity::from_bits(2)), None, 0);
        let mut transparent = ViewSortedRenderPhases::<Transparent3d>::default();
        transparent.prepare_for_new_frame(active);
        let mut post = ViewSortedRenderPhases::<PostOitResolveText3d>::default();
        post.prepare_for_new_frame(active);
        post.prepare_for_new_frame(inactive);
        prune_inactive_view_phases(&transparent, &mut post);
        assert!(post.contains_key(&active));
        assert!(!post.contains_key(&inactive));
    }

    #[test]
    fn stale_post_resolve_items_clear_when_source_disappears() {
        let mut world = World::new();
        let mut transparent = SortedRenderPhase::<Transparent3d>::default();
        let mut post = SortedRenderPhase::<PostOitResolveText3d>::default();
        let source = item(&mut world, 1.0);
        let main_entity = source.entity.1;
        post.add_retained(PostOitResolveText3d {
            source,
            draw_function: DrawFunctionId(2),
        });
        move_items_for_view(&mut transparent, &mut post, DrawFunctionId(2), [], |_| true);
        assert_eq!(
            post.items.len(),
            1,
            "unchanged retained draws survive another frame"
        );
        remove_dequeued_items(&mut post, [&main_entity]);
        move_items_for_view(&mut transparent, &mut post, DrawFunctionId(2), [], |_| true);
        assert!(post.items.is_empty());
    }
}
