//! Batched-records text geometry (`docs/hana_diegetic/glyph_instancing.md`):
//! routes every panel-text run into a per-[`PathBatchKey`] batch entity whose
//! vertex shader pulls per-glyph and per-run records from GPU tables.
//!
//! Panel-text batch schedule: [`update_panel_text_batches`]
//! writes glyph/run records before `TransformSystems::Propagate`;
//! [`write_batch_run_transforms`] copies propagated label transforms into run
//! records after it; [`update_batch_bounds`] hand-writes each batch entity's
//! `Aabb` and sort translation between `CalculateBounds` and
//! `CheckVisibility`; [`commit_batch_buffers`] uploads dirty record buffers
//! last, so extraction sees this frame's data.

use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::entity::EntityHashSet;
use bevy::ecs::system::SystemParam;
use bevy::light::NotShadowCaster;
use bevy::math::Vec3A;
use bevy::mesh::Indices;
use bevy::picking::Pickable;
use bevy::prelude::AlphaMode;
use bevy::prelude::AssetServer;
use bevy::prelude::Assets;
use bevy::prelude::Changed;
use bevy::prelude::ChildOf;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Handle;
use bevy::prelude::Mat4;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::Ref;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::RemovedComponents;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy::render::render_resource::PrimitiveTopology;
use bevy::render::storage::ShaderBuffer;
use bevy_kana::ToU32;
use bevy_kana::ToUsize;

use super::PanelTextLayout;
use super::PreparedPanelText;
use super::layout::PanelTextDrawZIndex;
use super::layout::PanelTextDrawZIndexRank;
use crate::cascade::HdrTextCoverageBias;
use crate::cascade::Resolved;
use crate::cascade::TextAlpha;
use crate::cascade::TextMaterial;
use crate::constants::MILLISECONDS_PER_SECOND;
use crate::layout::DrawBatchFamily;
use crate::layout::GlyphShadowMode;
use crate::layout::Lighting;
use crate::layout::ShadowCasting;
use crate::layout::Sidedness;
use crate::panel::DiegeticPanel;
use crate::panel::DiegeticPerfStats;
use crate::render;
use crate::render::AntiAlias;
use crate::render::BatchRenderLayers;
use crate::render::PathBatchKey;
use crate::render::PathBatchResources;
use crate::render::PathExtendedMaterial;
use crate::render::PathMaterialBuffers;
use crate::render::PathQuadRecord;
use crate::render::PathRenderRecord;
use crate::render::RenderMode;
use crate::render::VisualShadow;
use crate::render::analytic_paths::PathAtlasHandles;
use crate::render::batch_key;
use crate::render::batch_key::PipelineCompatibility;
use crate::render::batch_key::ResourceCompatibility;
use crate::render::draw_order::DrawOrderIndex;
use crate::render::material_table;
use crate::render::material_table::FrameMaterialSlotAppend;
use crate::render::material_table::FrameMaterialTableBuild;
use crate::render::material_table::FrameMaterialTableBuilder;
use crate::render::material_table::MaterialSlotCandidate;
use crate::render::material_table::MaterialSlotId;
use crate::render::material_table::RetainedBatchBufferUploads;
use crate::render::material_table::SdfPaintMaterial;
use crate::render::world_text::TextContent;
use crate::text;
use crate::text::GlyphCache;
use crate::text::GlyphQuadExtents;
use crate::text::PreparedTextRun;
use crate::text::RunStorageKey;
use crate::widgets::VisualOverrideIndex;
use crate::widgets::VisualSlotOverride;

/// Marker on every batch render entity, BRP-inspectable.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct DiegeticTextBatch;

/// Builds changed runs' glyph records, routes them through the batch store,
/// and reconciles batch entities and GPU assets to the store's state (spawn
/// on a key's first run, despawn on its last, mesh growth on a capacity
/// crossing — created, written, and swapped in the same frame).
///
/// Cascade inputs feeding [`PathBatchKey`] fields: each run's resolved
/// alpha / lighting / sidedness (with the global defaults for runs the
/// cascade has not seeded) plus the changed-this-frame run set that triggers
/// re-routing.
#[derive(SystemParam)]
pub(super) struct PathBatchKeyCascades<'w, 's> {
    alphas:                         Query<'w, 's, &'static Resolved<TextAlpha>, With<TextContent>>,
    lightings:                      Query<'w, 's, &'static Resolved<Lighting>, With<TextContent>>,
    sidednesses:                    Query<'w, 's, &'static Resolved<Sidedness>, With<TextContent>>,
    shadow_castings: Query<'w, 's, &'static Resolved<ShadowCasting>, With<TextContent>>,
    glyph_shadow_modes: Query<'w, 's, &'static Resolved<GlyphShadowMode>, With<TextContent>>,
    anti_aliases:                   Query<'w, 's, &'static Resolved<AntiAlias>, With<TextContent>>,
    materials: Query<'w, 's, &'static Resolved<TextMaterial>, With<TextContent>>,
    hdr_text_coverage_biases:
        Query<'w, 's, &'static Resolved<HdrTextCoverageBias>, With<TextContent>>,
    alpha_default:                  Res<'w, TextAlpha>,
    lighting_default:               Res<'w, Lighting>,
    sidedness_default:              Res<'w, Sidedness>,
    shadow_casting_default:         Res<'w, ShadowCasting>,
    glyph_shadow_mode_default:      Res<'w, GlyphShadowMode>,
    anti_alias_default:             Res<'w, AntiAlias>,
    hdr_text_coverage_bias_default: Res<'w, HdrTextCoverageBias>,
    placement_changed: Query<
        'w,
        's,
        Entity,
        (
            With<TextContent>,
            With<PreparedPanelText>,
            Or<(
                Changed<Resolved<TextAlpha>>,
                Changed<Resolved<Lighting>>,
                Changed<Resolved<Sidedness>>,
                Changed<Resolved<ShadowCasting>>,
                Changed<Resolved<GlyphShadowMode>>,
                Changed<Resolved<AntiAlias>>,
            )>,
        ),
    >,
    record_changed: Query<
        'w,
        's,
        Entity,
        (
            With<TextContent>,
            With<PreparedPanelText>,
            Changed<Resolved<HdrTextCoverageBias>>,
        ),
    >,
}

impl PathBatchKeyCascades<'_, '_> {
    /// Runs whose resolved placement or pipeline cascade value transitioned
    /// this frame. `Resolved<TextMaterial>` is excluded because
    /// `text_material_candidate_for_frame` recomputes table rows and
    /// compatibility every frame; scalar-only material changes are material
    /// row updates, while texture/pipeline changes are detected through
    /// `PathBatchKey` comparison.
    fn placement_changed_set(&self) -> EntityHashSet { self.placement_changed.iter().collect() }

    /// Runs whose per-record cascade value changed without changing glyph
    /// geometry or batch routing.
    fn record_changed_set(&self) -> EntityHashSet { self.record_changed.iter().collect() }

    fn alpha(&self, label: Entity) -> AlphaMode {
        self.alphas
            .get(label)
            .map_or(self.alpha_default.0, |resolved| resolved.0.0)
    }

    fn lighting(&self, label: Entity) -> Lighting {
        self.lightings
            .get(label)
            .map_or(*self.lighting_default, |resolved| resolved.0)
    }

    fn sidedness(&self, label: Entity) -> Sidedness {
        self.sidednesses
            .get(label)
            .map_or(*self.sidedness_default, |resolved| resolved.0)
    }

    fn visual_shadow(&self, label: Entity) -> VisualShadow {
        let shadow_casting = self
            .shadow_castings
            .get(label)
            .map_or(*self.shadow_casting_default, |resolved| resolved.0);
        let glyph_shadow_mode = self
            .glyph_shadow_modes
            .get(label)
            .map_or(*self.glyph_shadow_mode_default, |resolved| resolved.0);
        match shadow_casting {
            ShadowCasting::Off => VisualShadow::None,
            ShadowCasting::On => glyph_shadow_mode.into(),
        }
    }

    fn anti_alias(&self, label: Entity) -> AntiAlias {
        self.anti_aliases
            .get(label)
            .map_or(*self.anti_alias_default, |resolved| resolved.0)
    }

    fn hdr_text_coverage_bias(&self, label: Entity) -> HdrTextCoverageBias {
        self.hdr_text_coverage_biases
            .get(label)
            .map_or(*self.hdr_text_coverage_bias_default, |resolved| resolved.0)
    }

    fn material(&self, label: Entity, default: &TextMaterial) -> Handle<StandardMaterial> {
        self.materials
            .get(label)
            .map_or_else(|_| default.0.clone(), |resolved| resolved.0.0.clone())
    }
}

/// The query walks every run but touches only those whose text changed, whose
/// resolved cascade value changed (alpha / lighting / sidedness are batch-key
/// fields, so the run re-routes through `upsert_run` and moves batches when the
/// key differs), whose layout ordering changed, or that are not yet routed, so
/// the system is self-healing: a skipped frame (e.g. a glyph not yet packed)
/// re-routes on the next pass.
pub(super) fn update_panel_text_batches(
    runs: Query<
        (
            Entity,
            Ref<PreparedPanelText>,
            Ref<PanelTextLayout>,
            Ref<PanelTextDrawZIndex>,
            Ref<PanelTextDrawZIndexRank>,
            &ChildOf,
            &GlobalTransform,
            Option<&Visibility>,
        ),
        With<TextContent>,
    >,
    mut emptied_runs: RemovedComponents<PreparedPanelText>,
    panels: Query<(
        &DiegeticPanel,
        Option<&RenderLayers>,
        Option<&Visibility>,
        &GlobalTransform,
    )>,
    cascades: PathBatchKeyCascades,
    anti_alias: Res<AntiAlias>,
    mut backend: ResMut<GlyphCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<PathExtendedMaterial>>,
    standard_materials: Res<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    text_material_default: Res<TextMaterial>,
    mut storage_buffers: ResMut<Assets<ShaderBuffer>>,
    mut material_table: ResMut<FrameMaterialTableBuild>,
    visual_overrides: Res<VisualOverrideIndex>,
    mut perf: ResMut<DiegeticPerfStats>,
    mut commands: Commands,
) {
    let mesh_build_start = Instant::now();

    // An emptied or despawned run leaves its batch; the rebuild
    // re-derives the survivors' ranges.
    remove_emptied_panel_text_runs(&mut emptied_runs, &mut backend);

    // Cascade changes are inequality-guarded at the propagation pass, so this
    // set holds only real transitions; membership re-routes the run below.
    let cascade_changed = cascades.placement_changed_set();
    let record_changed = cascades.record_changed_set();

    // Upload the shared glyph atlas once before the run loop. A frame with no
    // glyph changes reuses existing handles, while live runs still append
    // current-frame material rows below.
    let atlas = backend.commit_glyph_atlas(&mut storage_buffers, &mut materials);

    for (
        label_entity,
        prepared,
        panel_text_child,
        z_index,
        z_index_rank,
        child_of,
        label_transform,
        label_visibility,
    ) in &runs
    {
        let storage_key = RunStorageKey::from(label_entity);
        let Ok((_panel, panel_layers, panel_visibility, panel_transform)) =
            panels.get(child_of.parent())
        else {
            backend.batch_store_mut().remove_run(storage_key);
            continue;
        };
        let hidden = is_hidden(label_visibility) || is_hidden(panel_visibility);

        let alpha_mode = cascades.alpha(label_entity);
        let lighting = cascades.lighting(label_entity);
        let sidedness = cascades.sidedness(label_entity);
        let shadow = cascades.visual_shadow(label_entity);
        let material = cascades.material(label_entity, &text_material_default);
        let slot_override = visual_overrides.get(child_of.parent(), panel_text_child.element_idx);
        let (fill_color, material) =
            apply_text_visual_override(slot_override, prepared.fill_color, material);
        let record_transform =
            text_run_record_transform(label_transform, panel_transform, slot_override);
        let Some(material_candidate) = text_material_candidate_for_frame(
            &material,
            fill_color,
            alpha_mode,
            lighting,
            sidedness,
            &standard_materials,
            &asset_server,
            &text_material_default,
        ) else {
            backend.batch_store_mut().remove_run(storage_key);
            continue;
        };
        if hidden {
            update_hidden_text_run(
                &mut backend,
                material_table.builder_mut(),
                storage_key,
                &prepared,
                &panel_text_child,
                material_candidate,
            );
            continue;
        }
        let batch_key = batch_key_for_run(
            panel_layers,
            *z_index,
            *z_index_rank,
            shadow,
            material_candidate.pipeline_compatibility,
            material_candidate.resource_compatibility.clone(),
        );
        apply_routed_text_run_update(
            RebuiltTextRunInput {
                backend: &mut backend,
                builder: material_table.builder_mut(),
                storage_key,
                batch_key,
                prepared: &prepared,
                panel_text_child: &panel_text_child,
                record_transform,
                anti_alias: cascades.anti_alias(label_entity),
                hdr_text_coverage_bias: cascades.hdr_text_coverage_bias(label_entity),
                material_candidate,
            },
            &prepared,
            &panel_text_child,
            &z_index,
            &z_index_rank,
            cascade_changed.contains(&label_entity),
            record_changed.contains(&label_entity),
        );
    }

    reconcile_batch_entities(ReconcileBatchEntities {
        atlas:           atlas.as_ref(),
        anti_alias:      *anti_alias,
        backend:         &mut backend,
        meshes:          &mut meshes,
        materials:       &mut materials,
        storage_buffers: &mut storage_buffers,
        commands:        &mut commands,
    });

    record_panel_text_batch_perf(&mut perf, mesh_build_start);
}

/// Records the completed text-batching duration and recomputes its total.
fn record_panel_text_batch_perf(perf: &mut DiegeticPerfStats, mesh_build_start: Instant) {
    perf.panel_text.mesh_build_ms =
        mesh_build_start.elapsed().as_secs_f32() * MILLISECONDS_PER_SECOND;
    perf.panel_text.total_ms = perf.panel_text.shape_ms + perf.panel_text.mesh_build_ms;
}

fn remove_emptied_panel_text_runs(
    emptied_runs: &mut RemovedComponents<PreparedPanelText>,
    backend: &mut GlyphCache,
) {
    for label_entity in emptied_runs.read() {
        backend
            .batch_store_mut()
            .remove_run(RunStorageKey::from(label_entity));
    }
}

fn update_hidden_text_run(
    backend: &mut GlyphCache,
    builder: &mut FrameMaterialTableBuilder,
    storage_key: RunStorageKey,
    prepared: &PreparedPanelText,
    panel_text_child: &PanelTextLayout,
    material_candidate: MaterialSlotCandidate,
) {
    backend.batch_store_mut().remove_run(storage_key);
    // Retain the row while omitting glyph records so toggling a tooltip does
    // not renumber later materials in the shared table.
    if build_glyph_records(
        backend,
        &prepared.prepared,
        prepared.clip_rect,
        panel_text_child,
    )
    .is_some_and(|glyphs| !glyphs.is_empty())
    {
        let _ = append_text_material_row(builder, storage_key, material_candidate);
    }
}

fn update_existing_text_run_material(
    builder: &mut FrameMaterialTableBuilder,
    backend: &mut GlyphCache,
    storage_key: RunStorageKey,
    material_candidate: MaterialSlotCandidate,
) {
    let Some(material_slot) = append_text_material_row(builder, storage_key, material_candidate)
    else {
        backend.batch_store_mut().remove_run(storage_key);
        return;
    };
    backend
        .batch_store_mut()
        .update_run_material(storage_key, material_slot);
}

struct RebuiltTextRunInput<'a> {
    backend:                &'a mut GlyphCache,
    builder:                &'a mut FrameMaterialTableBuilder,
    storage_key:            RunStorageKey,
    batch_key:              PathBatchKey,
    prepared:               &'a PreparedPanelText,
    panel_text_child:       &'a PanelTextLayout,
    record_transform:       Mat4,
    anti_alias:             AntiAlias,
    hdr_text_coverage_bias: HdrTextCoverageBias,
    material_candidate:     MaterialSlotCandidate,
}

fn apply_routed_text_run_update(
    input: RebuiltTextRunInput<'_>,
    prepared: &Ref<'_, PreparedPanelText>,
    panel_text_child: &Ref<'_, PanelTextLayout>,
    z_index: &Ref<'_, PanelTextDrawZIndex>,
    z_index_rank: &Ref<'_, PanelTextDrawZIndexRank>,
    cascade_changed: bool,
    render_record_changed: bool,
) {
    let key_changed = input
        .backend
        .batch_store()
        .key_for_run(input.storage_key)
        .is_none_or(|current| current != &input.batch_key);
    // A render-only prepared change leaves glyph geometry intact; every other
    // signal here re-derives the quads.
    let geometry_changed = (prepared.is_changed() && !prepared.render_only)
        || panel_text_child.is_changed()
        || z_index.is_changed()
        || z_index_rank.is_changed()
        || cascade_changed
        || key_changed;
    let render_only_changed = prepared.is_changed() || render_record_changed;
    apply_text_run_update(input, geometry_changed, render_only_changed);
}

/// Routes one run by what changed: a full glyph rebuild, a render-only record
/// refresh that reuses the existing quads, or a material-row write.
fn apply_text_run_update(
    input: RebuiltTextRunInput<'_>,
    geometry_changed: bool,
    render_only_changed: bool,
) {
    if geometry_changed {
        upsert_rebuilt_text_run(input);
    } else if render_only_changed {
        // Color or render-mode edit: glyph quads are unchanged, so refresh the
        // run record and material row instead of re-deriving identical geometry.
        refresh_text_run_record(RenderOnlyTextRunInput {
            backend:                input.backend,
            builder:                input.builder,
            storage_key:            input.storage_key,
            prepared:               input.prepared,
            panel_text_child:       input.panel_text_child,
            record_transform:       input.record_transform,
            anti_alias:             input.anti_alias,
            hdr_text_coverage_bias: input.hdr_text_coverage_bias,
            material_candidate:     input.material_candidate,
        });
    } else {
        update_existing_text_run_material(
            input.builder,
            input.backend,
            input.storage_key,
            input.material_candidate,
        );
    }
}

fn upsert_rebuilt_text_run(input: RebuiltTextRunInput<'_>) {
    let RebuiltTextRunInput {
        backend,
        builder,
        storage_key,
        batch_key,
        prepared,
        panel_text_child,
        record_transform,
        anti_alias,
        hdr_text_coverage_bias,
        material_candidate,
        ..
    } = input;
    // A glyph missing from the atlas means shaping has not packed it yet. An
    // unrouted run stays out and self-heals next frame; a run that was already
    // live must drop its now-stale records rather than keep rendering the prior
    // frame's glyphs.
    let Some(glyphs) = build_glyph_records(
        backend,
        &prepared.prepared,
        prepared.clip_rect,
        panel_text_child,
    ) else {
        if backend.batch_store().is_routed(storage_key) {
            backend.batch_store_mut().remove_run(storage_key);
        }
        return;
    };
    if glyphs.is_empty() {
        // Clipping removed every quad: drop the run so nothing renders.
        backend.batch_store_mut().remove_run(storage_key);
        return;
    }
    let Some(material_slot) = append_text_material_row(builder, storage_key, material_candidate)
    else {
        backend.batch_store_mut().remove_run(storage_key);
        return;
    };

    let record = run_record_for(
        prepared,
        panel_text_child,
        record_transform,
        anti_alias,
        hdr_text_coverage_bias,
        material_slot,
    );
    backend.batch_store_mut().upsert_run(
        batch_key,
        storage_key,
        DrawOrderIndex::from(panel_text_child.draw_ordinal),
        glyphs,
        record,
    );
}

struct RenderOnlyTextRunInput<'a> {
    backend:                &'a mut GlyphCache,
    builder:                &'a mut FrameMaterialTableBuilder,
    storage_key:            RunStorageKey,
    prepared:               &'a PreparedPanelText,
    panel_text_child:       &'a PanelTextLayout,
    record_transform:       Mat4,
    anti_alias:             AntiAlias,
    hdr_text_coverage_bias: HdrTextCoverageBias,
    material_candidate:     MaterialSlotCandidate,
}

/// Rewrites a routed run's `PathRenderRecord` and material-table row when only
/// render fields changed, reusing the glyph quads already in the batch.
fn refresh_text_run_record(input: RenderOnlyTextRunInput<'_>) {
    let RenderOnlyTextRunInput {
        backend,
        builder,
        storage_key,
        prepared,
        panel_text_child,
        record_transform,
        anti_alias,
        hdr_text_coverage_bias,
        material_candidate,
    } = input;
    let Some(material_slot) = append_text_material_row(builder, storage_key, material_candidate)
    else {
        backend.batch_store_mut().remove_run(storage_key);
        return;
    };
    let record = run_record_for(
        prepared,
        panel_text_child,
        record_transform,
        anti_alias,
        hdr_text_coverage_bias,
        material_slot,
    );
    backend
        .batch_store_mut()
        .update_run_record(storage_key, record);
}

const fn is_hidden(visibility: Option<&Visibility>) -> bool {
    matches!(visibility, Some(Visibility::Hidden))
}

/// Applies one widget visual-slot override to a routed text run.
///
/// Runs inside `update_panel_text_batches` before the frame material row is
/// appended: a replacement `text_color` patches only the run's material-table row
/// values, while a replacement `material` that changes
/// `PipelineCompatibility`/`ResourceCompatibility` changes the computed
/// `PathBatchKey` and `TextRunBatchStore::upsert_run` moves the run to its
/// compatible destination batch. A panel-local `offset` is applied to the
/// run's `PathRenderRecord` matrix instead, by [`text_run_record_transform`]
/// at record build and by [`write_batch_run_transforms`] after propagation.
fn apply_text_visual_override(
    slot_override: Option<&VisualSlotOverride>,
    fill_color: Color,
    material: Handle<StandardMaterial>,
) -> (Color, Handle<StandardMaterial>) {
    let Some(slot_override) = slot_override else {
        return (fill_color, material);
    };
    (
        slot_override.text_color.unwrap_or(fill_color),
        slot_override.material.clone().unwrap_or(material),
    )
}

/// World matrix for one run's `PathRenderRecord`: the label's
/// `GlobalTransform` matrix plus any slot-override `offset` composed through
/// the owning panel's global transform via [`offset_record_matrix`]. The
/// text-run entity `Transform` and `PanelTextLayout` payload stay untouched.
fn text_run_record_transform(
    label_transform: &GlobalTransform,
    panel_transform: &GlobalTransform,
    slot_override: Option<&VisualSlotOverride>,
) -> Mat4 {
    let matrix = label_transform.to_matrix();
    slot_override
        .and_then(|slot_override| slot_override.offset)
        .map_or(matrix, |offset| {
            offset_record_matrix(matrix, panel_transform, offset)
        })
}

/// Adds a panel-local `offset` to a world `matrix` by rotating and scaling
/// the offset vector through the panel's global transform, so an offset on a
/// rotated or scaled panel moves the record in the panel plane rather than
/// along world axes.
fn offset_record_matrix(matrix: Mat4, panel_transform: &GlobalTransform, offset: Vec2) -> Mat4 {
    let mut matrix = matrix;
    matrix.w_axis += panel_transform
        .affine()
        .transform_vector3(offset.extend(0.0))
        .extend(0.0);
    matrix
}

fn text_material_candidate_for_frame(
    handle: &Handle<StandardMaterial>,
    fill_color: Color,
    alpha_mode: AlphaMode,
    lighting: Lighting,
    sidedness: Sidedness,
    standard_materials: &Assets<StandardMaterial>,
    asset_server: &AssetServer,
    text_material_default: &TextMaterial,
) -> Option<MaterialSlotCandidate> {
    let base = render::material_asset_for_frame(
        standard_materials,
        asset_server,
        handle,
        &text_material_default.0,
    )?;
    let base = strip_tangent_dependent_maps(base);
    Some(render::analytic_material_slot_candidate(
        &base, fill_color, alpha_mode, lighting, sidedness,
    ))
}

/// Glyph quads carry position and UVs but no tangents, so normal and parallax
/// (depth) maps would sample against an undefined tangent basis and skew the
/// lighting. Drop them here so a text material that sets either still lights
/// correctly from its remaining channels instead of rendering wrong.
fn strip_tangent_dependent_maps(base: &StandardMaterial) -> StandardMaterial {
    let mut material = base.clone();
    material.normal_map_texture = None;
    material.depth_map = None;
    material
}

fn append_text_material_row(
    builder: &mut FrameMaterialTableBuilder,
    storage_key: RunStorageKey,
    candidate: MaterialSlotCandidate,
) -> Option<MaterialSlotId> {
    match builder.upsert_values(storage_key.into(), candidate.values) {
        FrameMaterialSlotAppend::Appended(slot) => Some(slot),
        FrameMaterialSlotAppend::DroppedLimit => None,
    }
}

fn batch_key_for_run(
    panel_layers: Option<&RenderLayers>,
    z_index: PanelTextDrawZIndex,
    z_index_rank: PanelTextDrawZIndexRank,
    shadow: VisualShadow,
    pipeline_compatibility: PipelineCompatibility,
    resource_compatibility: ResourceCompatibility,
) -> PathBatchKey {
    PathBatchKey {
        z_index: z_index.0,
        z_index_rank: z_index_rank.0,
        batch_family: DrawBatchFamily::Text,
        shadow,
        layers: BatchRenderLayers(panel_layers.cloned().unwrap_or(RenderLayers::layer(0))),
        pipeline_compatibility,
        resource_compatibility,
    }
}

fn run_record_for(
    prepared: &PreparedPanelText,
    panel_text_child: &PanelTextLayout,
    record_transform: Mat4,
    anti_alias: AntiAlias,
    hdr_text_coverage_bias: HdrTextCoverageBias,
    material: MaterialSlotId,
) -> PathRenderRecord {
    PathRenderRecord {
        // Pre-propagation snapshot; write_batch_run_transforms corrects it
        // after TransformSystems::Propagate the same frame.
        transform:          record_transform,
        material:           material.into(),
        render_mode:        u32::from(RenderMode::from(prepared.render_mode)),
        clip_depth_nudge:   panel_text_child.depth_bias,
        oit_depth_offset:   panel_text_child.oit_depth_offset,
        aa_flags:           anti_alias.aa_flags(),
        text_coverage_bias: hdr_text_coverage_bias.shader_value(),
        text_em_height:     prepared.em_height,
    }
}

/// Inputs for [`reconcile_batch_entities`].
struct ReconcileBatchEntities<'a, 'w, 's> {
    atlas:           Option<&'a PathAtlasHandles>,
    anti_alias:      AntiAlias,
    backend:         &'a mut GlyphCache,
    meshes:          &'a mut Assets<Mesh>,
    materials:       &'a mut Assets<PathExtendedMaterial>,
    storage_buffers: &'a mut Assets<ShaderBuffer>,
    commands:        &'a mut Commands<'w, 's>,
}

/// Reconciles batch entities and GPU assets to the store's state: despawns
/// emptied batches, spawns entities + assets for new keys, and re-creates the
/// inert mesh on a capacity crossing.
fn reconcile_batch_entities(input: ReconcileBatchEntities<'_, '_, '_>) {
    let ReconcileBatchEntities {
        atlas,
        anti_alias,
        backend,
        meshes,
        materials,
        storage_buffers,
        commands,
    } = input;
    for entity in backend.batch_store_mut().take_empty_batches() {
        commands.entity(entity).despawn();
    }
    let mut to_create = Vec::new();
    let mut to_grow = Vec::new();
    for (key, batch) in backend.batch_store().batches() {
        match &batch.gpu {
            None => to_create.push(key.clone()),
            Some(gpu)
                if batch.path_record_count() > gpu.capacity
                    || batch.run_count().to_u32() > gpu.run_capacity =>
            {
                to_grow.push(key.clone());
            },
            Some(_) => {},
        }
    }
    if let Some(atlas) = atlas {
        for key in to_create {
            spawn_batch_entity(SpawnBatchEntity {
                key: &key,
                atlas,
                anti_alias,
                backend: &mut *backend,
                meshes: &mut *meshes,
                materials: &mut *materials,
                storage_buffers: &mut *storage_buffers,
                commands: &mut *commands,
            });
        }
    }
    for key in to_grow {
        grow_batch_assets(&key, backend, meshes, materials, storage_buffers, commands);
    }
    refresh_batch_material_depth_biases(backend, materials);
}

/// Copies each routed label's propagated `GlobalTransform` into its
/// `PathRenderRecord` slot, composing any widget slot-override `offset`
/// through the owning panel's global transform. A label whose transform did
/// not change is recomputed only when the `VisualOverrideIndex` changed this
/// frame, and the store dirties the run table only when the matrix actually
/// changed, so a static frame uploads nothing.
pub(super) fn write_batch_run_transforms(
    labels: Query<
        (Entity, Ref<GlobalTransform>, &ChildOf, &PanelTextLayout),
        (With<TextContent>, With<PreparedPanelText>),
    >,
    panels: Query<&GlobalTransform, With<DiegeticPanel>>,
    visual_overrides: Res<VisualOverrideIndex>,
    mut backend: ResMut<GlyphCache>,
) {
    let overrides_changed = visual_overrides.is_changed();
    for (label_entity, transform, child_of, panel_text_child) in &labels {
        if !transform.is_changed() && !overrides_changed {
            continue;
        }
        let mut matrix = transform.to_matrix();
        if let Some(offset) = visual_overrides
            .get(child_of.parent(), panel_text_child.element_idx)
            .and_then(|slot_override| slot_override.offset)
            && let Ok(panel_transform) = panels.get(child_of.parent())
        {
            matrix = offset_record_matrix(matrix, panel_transform, offset);
        }
        backend
            .batch_store_mut()
            .update_run_transform(RunStorageKey::from(label_entity), matrix);
    }
}

/// Hand-writes each dirty batch's bounds: the entity's translation moves to
/// the union's world center (the `Transparent3d` sort distance) and its
/// local-space `Aabb` gets the union's half-extents. The batch entity carries
/// `NoAutoAabb` — `CalculateBounds` would otherwise install a zero-extent box
/// computed from the inert mesh's zeroed positions on every growth frame.
pub(super) fn update_batch_bounds(
    mut backend: ResMut<GlyphCache>,
    mut batch_entities: Query<
        (&mut Transform, &mut GlobalTransform, &mut Aabb),
        With<DiegeticTextBatch>,
    >,
) {
    for (_, batch) in backend.batch_store_mut().batches_mut() {
        if !batch.bounds_are_dirty() {
            continue;
        }
        let Some(entity) = batch.entity else {
            continue;
        };
        let Ok((mut transform, mut global, mut aabb)) = batch_entities.get_mut(entity) else {
            continue;
        };
        let Some((min, max)) = batch.world_bounds() else {
            continue;
        };
        let center = (min + max) * 0.5;
        // The batch entity has no parent and propagation already ran, so the
        // pair is written directly; the vertex shader ignores it (placement
        // comes from run records) — this is sort/culling metadata only.
        *transform = Transform::from_translation(center);
        *global = GlobalTransform::from(*transform);
        *aabb = Aabb {
            center:       Vec3A::ZERO,
            half_extents: Vec3A::from((max - min) * 0.5),
        };
        batch.clear_bounds_dirty();
    }
}

/// Uploads each batch's dirty record buffers — one `set_data` per dirty
/// buffer per batch — and publishes the batch counters. The split dirty flags
/// keep the uploads minimal: a transform-only frame uploads only the run
/// table, a same-count text edit only the instance buffer, an unchanged frame
/// nothing.
///
/// Every payload is padded to the buffer's capacity so its byte length never
/// changes between growths. The shared retained-upload queue writes those bytes
/// into the existing wgpu buffer in the render world, so the material's bind
/// group remains valid without asset replacement or re-preparation (see
/// [`PathBatchResources`](crate::render::PathBatchResources)).
pub(super) fn commit_batch_buffers(
    mut backend: ResMut<GlyphCache>,
    mut staged_uploads: ResMut<RetainedBatchBufferUploads>,
    mut perf: ResMut<DiegeticPerfStats>,
) {
    let mut batches = 0_usize;
    let mut runs = 0_usize;
    let mut glyph_records = 0_usize;
    let mut instance_uploads = 0_usize;
    let mut run_table_uploads = 0_usize;
    perf.text_breakdown.clear();
    for (key, batch) in backend.batch_store_mut().batches_mut() {
        batches += 1;
        runs += batch.run_count();
        glyph_records += batch.path_record_count().to_usize();
        perf.text_breakdown.push(render::batch_summary(
            key.z_index,
            &key.layers,
            key.shadow,
            &key.pipeline_compatibility,
            &key.resource_compatibility,
            batch.path_record_count(),
        ));
        if batch.gpu.is_none() {
            continue;
        }
        let instances_payload = batch.path_quads_are_dirty().then(|| {
            let capacity = batch.gpu.as_ref().map_or(0, |gpu| gpu.capacity);
            padded_glyph_records(batch.path_records(), capacity)
        });
        let run_table_payload = batch.render_records_are_dirty().then(|| {
            let run_capacity = batch.gpu.as_ref().map_or(0, |gpu| gpu.run_capacity);
            padded_run_records(batch.run_records(), run_capacity)
        });
        batch.clear_path_quad_dirty();
        batch.clear_render_record_dirty();
        let Some(gpu) = &batch.gpu else {
            continue;
        };
        if let Some(data) = instances_payload {
            let handle = gpu.instances.clone();
            material_table::stage_retained_batch_buffer_upload(
                &mut staged_uploads,
                &handle,
                ShaderBuffer::from(data),
            );
            instance_uploads += 1;
        }
        if let Some(data) = run_table_payload {
            let handle = gpu.run_table.clone();
            material_table::stage_retained_batch_buffer_upload(
                &mut staged_uploads,
                &handle,
                ShaderBuffer::from(data),
            );
            run_table_uploads += 1;
        }
    }
    perf.batch.batches = batches.into();
    perf.batch.runs = runs.into();
    perf.batch.glyph_records = glyph_records.into();
    perf.batch.instance_uploads = instance_uploads.into();
    perf.batch.run_table_uploads = run_table_uploads.into();
}

/// Glyph records padded to `capacity` with zero-size quads: every corner of a
/// padding quad coincides, so it rasterizes nothing, and the buffer's byte
/// length stays constant between growths.
fn padded_glyph_records(records: &[PathQuadRecord], capacity: u32) -> Vec<PathQuadRecord> {
    let mut padded = Vec::with_capacity(capacity.to_usize());
    padded.extend_from_slice(records);
    padded.resize(
        capacity.to_usize().max(records.len()),
        PathQuadRecord {
            rect_min:          Vec2::ZERO,
            rect_size:         Vec2::ZERO,
            uv_min:            Vec2::ZERO,
            uv_size:           Vec2::ZERO,
            box_uv_min:        Vec2::ZERO,
            box_uv_size:       Vec2::ZERO,
            packed_path_index: 0,
            render_index:      0,
            box_uv_flip_x:     0,
        },
    );
    padded
}

/// Run records padded to `run_capacity`. Padding slots are never referenced —
/// no live glyph record carries their index, and zero-size padding quads
/// produce no fragments — so every field can be zero (`render_mode` 0 is
/// deliberately neither `Text` nor `PunchOut`).
fn padded_run_records(records: &[PathRenderRecord], run_capacity: u32) -> Vec<PathRenderRecord> {
    let mut padded = Vec::with_capacity(run_capacity.to_usize());
    padded.extend_from_slice(records);
    padded.resize(
        run_capacity.to_usize().max(records.len()),
        PathRenderRecord {
            transform:          Mat4::ZERO,
            material:           SdfPaintMaterial::NotAuthored.to_gpu(),
            render_mode:        0,
            clip_depth_nudge:   0.0,
            oit_depth_offset:   0.0,
            aa_flags:           0,
            text_coverage_bias: 0.0,
            text_em_height:     0.0,
        },
    );
    padded
}

/// Builds one run's glyph instance records against the shared atlas, with
/// each quad's padded rect and UVs clipped by `glyph_quad_extents`.
/// `render_index` is `0` on every record — the batch store stamps it at rebuild.
/// Returns `None` when a glyph is not yet packed.
fn build_glyph_records(
    cache: &GlyphCache,
    prepared: &PreparedTextRun,
    clip_rect: Option<[f32; 4]>,
    panel_text_child: &PanelTextLayout,
) -> Option<Vec<PathQuadRecord>> {
    let mut records = Vec::with_capacity(prepared.glyph_count());
    let (box_min, box_size) = text_box_min_size(panel_text_child);
    for glyph in prepared.glyphs() {
        let packed_path_index = cache.packed_path_index(glyph.key())?;
        let Some(extents) = text::glyph_quad_extents(*glyph, 1.0, clip_rect) else {
            continue;
        };
        let (box_uv_min, box_uv_size) = glyph_box_uv(&extents, box_min, box_size);
        records.push(PathQuadRecord {
            rect_min: Vec2::new(extents.left, extents.bottom),
            rect_size: Vec2::new(extents.right - extents.left, extents.top - extents.bottom),
            uv_min: Vec2::new(extents.uv_left, extents.uv_top),
            uv_size: Vec2::new(
                extents.uv_right - extents.uv_left,
                extents.uv_bottom - extents.uv_top,
            ),
            box_uv_min,
            box_uv_size,
            packed_path_index,
            render_index: 0,
            box_uv_flip_x: 0,
        });
    }
    Some(records)
}

fn text_box_min_size(panel_text_child: &PanelTextLayout) -> (Vec2, Vec2) {
    let bounds = panel_text_child.bounds;
    let size = Vec2::new(
        bounds.width * panel_text_child.scale_x,
        bounds.height * panel_text_child.scale_y,
    );
    let min = Vec2::new(
        bounds
            .x
            .mul_add(panel_text_child.scale_x, -panel_text_child.anchor_x),
        (bounds.y + bounds.height).mul_add(-panel_text_child.scale_y, panel_text_child.anchor_y),
    );
    (min, size)
}

fn glyph_box_uv(extents: &GlyphQuadExtents, box_min: Vec2, box_size: Vec2) -> (Vec2, Vec2) {
    let inv_size = Vec2::new(
        box_size.x.max(f32::EPSILON).recip(),
        box_size.y.max(f32::EPSILON).recip(),
    );
    let box_top = box_min.y + box_size.y;
    let left = ((extents.left - box_min.x) * inv_size.x).clamp(0.0, 1.0);
    let right = ((extents.right - box_min.x) * inv_size.x).clamp(0.0, 1.0);
    let top = ((box_top - extents.top) * inv_size.y).clamp(0.0, 1.0);
    let bottom = ((box_top - extents.bottom) * inv_size.y).clamp(0.0, 1.0);
    (Vec2::new(left, top), Vec2::new(right - left, bottom - top))
}

/// Inert capacity-sized batch mesh: zeroed `POSITION` / `UV_0` / `UV_1`
/// (values unread — the layout switches the `VERTEX_UVS_A/B` pipeline defs
/// on) plus the static per-quad `U32` index pattern winding each quad
/// `base, base+3, base+2, base, base+2, base+1`.
fn inert_batch_mesh(capacity: u32) -> Mesh {
    let vertex_count = capacity.to_usize() * 4;
    let mut indices = Vec::with_capacity(capacity.to_usize() * 6);
    for quad in 0..capacity {
        let base = quad * 4;
        indices.extend([base, base + 3, base + 2, base, base + 2, base + 1]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0_f32; 3]; vertex_count]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0_f32; 2]; vertex_count]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0_f32; 2]; vertex_count]);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Inputs for [`spawn_batch_entity`].
struct SpawnBatchEntity<'a, 'w, 's> {
    key:             &'a PathBatchKey,
    atlas:           &'a PathAtlasHandles,
    anti_alias:      AntiAlias,
    backend:         &'a mut GlyphCache,
    meshes:          &'a mut Assets<Mesh>,
    materials:       &'a mut Assets<PathExtendedMaterial>,
    storage_buffers: &'a mut Assets<ShaderBuffer>,
    commands:        &'a mut Commands<'w, 's>,
}

/// Spawns one batch's render entity with its GPU assets: record buffers
/// created from the current store contents, an inert mesh at the rounded-up
/// capacity, and the per-batch vertex-pulling material.
fn spawn_batch_entity(input: SpawnBatchEntity<'_, '_, '_>) {
    let SpawnBatchEntity {
        key,
        atlas,
        anti_alias,
        backend,
        meshes,
        materials,
        storage_buffers,
        commands,
    } = input;
    let Some(batch) = backend.batch_store().get(key) else {
        return;
    };
    let capacity = batch.path_record_count().next_power_of_two();
    let run_capacity = batch.run_count().to_u32().next_power_of_two();
    let glyph_records = padded_glyph_records(batch.path_records(), capacity);
    let run_records = padded_run_records(batch.run_records(), run_capacity);

    let instances = storage_buffers.add(ShaderBuffer::from(glyph_records));
    let run_table = storage_buffers.add(ShaderBuffer::from(run_records));
    let mesh = meshes.add(inert_batch_mesh(capacity));
    let material = materials.add(batch_material(BatchMaterialInput {
        key,
        atlas,
        instances: instances.clone(),
        run_table: run_table.clone(),
        anti_alias,
    }));

    let mut batch_entity = commands.spawn((
        DiegeticTextBatch,
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material.clone()),
        Pickable::IGNORE,
        // The union system owns this Aabb; CalculateBounds must not replace
        // it with a zero-extent box from the inert mesh's zeroed positions.
        NoAutoAabb,
        Aabb::default(),
        key.layers.0.clone(),
    ));
    if key.shadow == VisualShadow::None {
        batch_entity.insert(NotShadowCaster);
    }
    let entity = batch_entity.id();

    if let Some(batch) = backend.batch_store_mut().get_mut(key) {
        batch.entity = Some(entity);
        batch.gpu = Some(PathBatchResources {
            instances,
            run_table,
            mesh,
            material,
            capacity,
            run_capacity,
        });
        // The buffers were created from the current records; only later
        // writes (e.g. this frame's post-propagation transform pass) need an
        // upload. Bounds dirtiness stays set so the union system places the
        // entity this frame.
        batch.clear_path_quad_dirty();
        batch.clear_render_record_dirty();
    }
}

/// Grows a batch past its capacity: new padded record buffers and a new inert
/// mesh at the doubled capacities, the mesh swapped onto the entity and the
/// material's buffer handles rewritten in place. `TextRenderPlugin` creates
/// the replacement assets before `AssetEventSystems`, and `MaterialTablePlugin`
/// orders GPU buffer preparation before `PathExtendedMaterial` preparation so
/// the replacement mesh and buffers are available in the same extracted frame.
fn grow_batch_assets(
    key: &PathBatchKey,
    backend: &mut GlyphCache,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PathExtendedMaterial>,
    storage_buffers: &mut Assets<ShaderBuffer>,
    commands: &mut Commands,
) {
    let Some(batch) = backend.batch_store_mut().get_mut(key) else {
        return;
    };
    let Some(entity) = batch.entity else {
        return;
    };
    let required = batch.path_record_count();
    let run_required = batch.run_count().to_u32();
    let (Some(current_capacity), Some(current_run_capacity)) = (
        batch.gpu.as_ref().map(|gpu| gpu.capacity),
        batch.gpu.as_ref().map(|gpu| gpu.run_capacity),
    ) else {
        return;
    };
    let mut capacity = current_capacity.max(1);
    while capacity < required {
        capacity *= 2;
    }
    let mut run_capacity = current_run_capacity.max(1);
    while run_capacity < run_required {
        run_capacity *= 2;
    }

    let instances = storage_buffers.add(ShaderBuffer::from(padded_glyph_records(
        batch.path_records(),
        capacity,
    )));
    let run_table = storage_buffers.add(ShaderBuffer::from(padded_run_records(
        batch.run_records(),
        run_capacity,
    )));
    let mesh = meshes.add(inert_batch_mesh(capacity));
    commands.entity(entity).insert(Mesh3d(mesh.clone()));

    let Some(gpu) = &mut batch.gpu else {
        return;
    };
    if let Some(mut material) = materials.get_mut(&gpu.material) {
        render::set_path_material_record_buffers(
            &mut material,
            instances.clone(),
            run_table.clone(),
        );
    }
    gpu.instances = instances;
    gpu.run_table = run_table;
    gpu.mesh = mesh;
    gpu.capacity = capacity;
    gpu.run_capacity = run_capacity;
    // The new buffers were created from the current records.
    batch.clear_path_quad_dirty();
    batch.clear_render_record_dirty();
}

/// Inputs for [`batch_material`].
struct BatchMaterialInput<'a> {
    key:        &'a PathBatchKey,
    atlas:      &'a PathAtlasHandles,
    instances:  Handle<ShaderBuffer>,
    run_table:  Handle<ShaderBuffer>,
    anti_alias: AntiAlias,
}

/// Builds one batch's material from resource/pipeline compatibility and atlas
/// buffers. Scalar PBR values are read per `PathRenderRecord::material` from
/// the frame material table.
fn batch_material(input: BatchMaterialInput<'_>) -> PathExtendedMaterial {
    let BatchMaterialInput {
        key,
        atlas,
        instances,
        run_table,
        anti_alias,
    } = input;
    let base = render::default_panel_material();
    let mut base = batch_key::apply_resource_compatibility_to_standard_material(
        &base,
        &key.resource_compatibility,
    );
    batch_key::apply_pipeline_compatibility_to_standard_material(
        &mut base,
        key.pipeline_compatibility,
    );
    base.alpha_mode = batch_gpu_alpha_mode(key.pipeline_compatibility.alpha.into());
    base.depth_bias = key.z_index_rank.screen_depth_bias().get();
    PathExtendedMaterial {
        base,
        extension: render::analytic_paths::vertex_pull(
            RenderMode::Text,
            0.0,
            anti_alias,
            PathMaterialBuffers {
                curves: atlas.curves.clone(),
                bands: atlas.bands.clone(),
                path_records: atlas.path_records.clone(),
                instances,
                run_records: run_table,
            },
        ),
    }
}

fn refresh_batch_material_depth_biases(
    backend: &mut GlyphCache,
    materials: &mut Assets<PathExtendedMaterial>,
) {
    for (key, batch) in backend.batch_store_mut().batches_mut() {
        let Some(gpu) = &batch.gpu else {
            continue;
        };
        let Some(mut material) = materials.get_mut(&gpu.material) else {
            continue;
        };
        let depth_bias = key.z_index_rank.screen_depth_bias().get();
        if material.base.depth_bias.to_bits() != depth_bias.to_bits() {
            material.base.depth_bias = depth_bias;
        }
    }
}

/// Maps a batch's authored alpha mode to the one written on the GPU material.
/// Invariant: [`PathBatchKey::pipeline_compatibility`] always keeps the user's
/// authored alpha mode — only the material differs, and only where bevy's
/// pipeline routing requires it.
///
/// `Opaque` becomes `Mask(0.0)` for every batch: opaque meshes take bevy's
/// depth-only pipelines — the camera depth/normal prepass as well as the
/// shadow pass — which strip the material bind group from the pipeline
/// layout, and the vertex-pull stage reads bindings 104/105 from that group,
/// so pipeline creation fails wgpu validation. The camera prepass is why this
/// remap is unconditional while `sdf_batch_alpha_mode` gates its remap on
/// `VisualShadow::Cast`: a non-casting opaque SDF batch enters no depth-only
/// pass, but opaque text would still hit the camera prepass. `Mask(0.0)`
/// renders the same pixels (cutoff 0 never discards by alpha; the coverage
/// discards cut the glyph outlines; depth writes, nothing blends) and its
/// `MAY_DISCARD` pipelines keep the material bind group.
const fn batch_gpu_alpha_mode(authored: AlphaMode) -> AlphaMode {
    match authored {
        AlphaMode::Opaque => AlphaMode::Mask(0.0),
        other => other,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should fail loudly when fixture batches are missing"
)]
mod tests {
    use std::collections::hash_map::DefaultHasher;
    use std::f32::consts::FRAC_PI_2;
    use std::hash::Hasher;
    use std::sync::Arc;

    use bevy::asset::AssetPlugin;
    use bevy::ecs::change_detection::Tick;
    use bevy::image::Image;
    use bevy::picking::hover::PickingInteraction;
    use bevy::prelude::App;
    use bevy::prelude::AssetApp;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::PostUpdate;
    use bevy::prelude::Quat;
    use bevy::prelude::TransformPlugin;
    use bevy::prelude::TransformSystems;
    use bevy::prelude::Vec3;
    use bevy::prelude::Vec4;
    use bevy_kana::ToF32;

    use super::*;
    use crate::Appearance;
    use crate::Border;
    use crate::Mm;
    use crate::PanelWidget;
    use crate::cascade;
    use crate::cascade::CascadeEntityCommandsExt;
    use crate::cascade::TextMaterial;
    use crate::constants::MONOSPACE_WIDTH_RATIO;
    use crate::layout::DrawZIndex;
    use crate::layout::El;
    use crate::layout::GlyphShadowMode;
    use crate::layout::LayoutBuilder;
    use crate::layout::LayoutTree;
    use crate::layout::Text;
    use crate::layout::TextDimensions;
    use crate::layout::TextMeasure;
    use crate::layout::TextStyle;
    use crate::panel::ComputedDiegeticPanel;
    use crate::panel::DiegeticPanelCommands;
    use crate::panel::HeadlessLayoutPlugin;
    use crate::render::constants;
    use crate::render::material_table::BatchAssetWrites;
    use crate::render::material_table::MaterialSlotValues;
    use crate::render::material_table::MaterialTableAppendReady;
    use crate::render::material_table::MaterialTablePlugin;
    use crate::render::panel_text::alpha;
    use crate::render::panel_text::glyph_cascade;
    use crate::render::panel_text::reify;
    use crate::render::panel_text::shaping;
    use crate::render::text_shaping::TextShapingContext;
    use crate::text::DiegeticTextMeasurer;
    use crate::text::FontRegistry;
    use crate::widgets::VisualElementCapabilities;
    use crate::widgets::VisualSlotId;
    use crate::widgets::VisualSlotOverride;
    use crate::widgets::WidgetVisualOverrides;
    use crate::widgets::WidgetVisualSlots;
    use crate::widgets::WidgetsPlugin;

    const LOWERED_LEVEL: DrawZIndex = DrawZIndex(-1);
    const NON_INTERSECTING_CLIP_RECT: [f32; 4] = [f32::MAX; 4];
    const RAISED_LEVEL: DrawZIndex = DrawZIndex(1);
    const TEXT_OVERRIDE_COLOR: Color = Color::srgb(0.9, 0.1, 0.3);
    const TEXT_SLOT: VisualSlotId = VisualSlotId::new(9);
    /// Panel-local offset with distinct axis values so a rotated panel maps
    /// it to a visibly different world delta.
    const TEXT_SLOT_OFFSET: Vec2 = Vec2::new(4.0, -2.0);

    fn monospace_measurer() -> DiegeticTextMeasurer {
        DiegeticTextMeasurer {
            measure_fn: Arc::new(|text: &str, measure: &TextMeasure| {
                let char_width = measure.size * MONOSPACE_WIDTH_RATIO;
                let width = text
                    .lines()
                    .map(|line| line.chars().count().to_f32() * char_width)
                    .fold(0.0_f32, f32::max);
                let line_count = text.lines().count().max(1).to_f32();
                TextDimensions {
                    width,
                    height: measure.size * line_count,
                    line_height: measure.size,
                }
            }),
        }
    }

    /// App with the full panel-text pipeline — the production registration's
    /// ordering, minus the visibility-set edges (headless: no visibility
    /// systems run).
    fn pipeline_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .add_plugins(MaterialTablePlugin)
            .add_plugins(TransformPlugin)
            .insert_resource(monospace_measurer())
            .add_plugins(HeadlessLayoutPlugin)
            .add_plugins(cascade::cascade_plugin::<TextAlpha>())
            .add_plugins(cascade::cascade_plugin::<TextMaterial>())
            .add_plugins(cascade::cascade_plugin::<Lighting>())
            .add_plugins(cascade::cascade_plugin::<Sidedness>())
            .add_plugins(cascade::cascade_plugin::<GlyphShadowMode>())
            .insert_resource(FontRegistry::new().expect("embedded font should parse"))
            .init_resource::<TextShapingContext>()
            .init_resource::<GlyphCache>()
            .init_resource::<AntiAlias>()
            .init_resource::<VisualOverrideIndex>()
            .init_asset::<Mesh>()
            .init_asset::<ShaderBuffer>()
            .init_asset::<PathExtendedMaterial>()
            .add_observer(alpha::seed_panel_text_child_alpha)
            .add_observer(glyph_cascade::seed_panel_text_child_glyph)
            .add_systems(
                PostUpdate,
                (
                    reify::reify_text_entities,
                    shaping::shape_panel_text_children.after(reify::reify_text_entities),
                    update_panel_text_batches
                        .after(shaping::shape_panel_text_children)
                        .after(MaterialTableAppendReady)
                        .in_set(BatchAssetWrites)
                        .before(TransformSystems::Propagate),
                    write_batch_run_transforms.after(TransformSystems::Propagate),
                    update_batch_bounds.after(write_batch_run_transforms),
                    commit_batch_buffers
                        .after(update_panel_text_batches)
                        .after(write_batch_run_transforms),
                ),
            );
        render::seed_default_material_cascades(&mut app);
        app
    }

    fn two_text_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(("Alpha", TextStyle::new(10.0)));
        builder.text(("Beta", TextStyle::new(10.0)));
        builder.build()
    }

    fn one_text_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(("Alpha", TextStyle::new(10.0)));
        builder.build()
    }

    fn one_text_tree_with_style(style: TextStyle) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(("Alpha", style));
        builder.build()
    }

    fn material_with_metallic(app: &mut App, metallic: f32) -> Handle<StandardMaterial> {
        app.world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                metallic,
                ..Default::default()
            })
    }

    fn material_with_texture(app: &mut App, texture: Handle<Image>) -> Handle<StandardMaterial> {
        app.world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color_texture: Some(texture),
                alpha_mode: AlphaMode::Blend,
                ..Default::default()
            })
    }

    fn spawn_panel(app: &mut App, tree: LayoutTree) -> Entity {
        app.world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .with_tree(tree)
                    .build()
                    .expect("panel should build"),
            )
            .id()
    }

    fn settle(app: &mut App) {
        for _ in 0..4 {
            app.update();
        }
    }

    fn batch_entities(app: &mut App) -> Vec<Entity> {
        let mut state = app
            .world_mut()
            .query_filtered::<Entity, With<DiegeticTextBatch>>();
        state.iter(app.world()).collect()
    }

    fn label_entities(app: &mut App) -> Vec<Entity> {
        let mut state = app
            .world_mut()
            .query_filtered::<Entity, With<TextContent>>();
        state.iter(app.world()).collect()
    }

    fn store_stats(app: &App) -> (usize, usize, usize) {
        let store = app.world().resource::<GlyphCache>().batch_store();
        let batches = store.batches().count();
        let runs: usize = store.batches().map(|(_, batch)| batch.run_count()).sum();
        let glyphs: usize = store
            .batches()
            .map(|(_, batch)| batch.path_record_count().to_usize())
            .sum();
        (batches, runs, glyphs)
    }

    fn text_instance_buffer(app: &App) -> Handle<ShaderBuffer> {
        let store = app.world().resource::<GlyphCache>().batch_store();
        let (_, batch) = store.batches().next().expect("one text batch should exist");
        batch
            .gpu
            .as_ref()
            .expect("the settled text batch should own GPU assets")
            .instances
            .clone()
    }

    fn frame_material_row_count(app: &App) -> usize {
        app.world()
            .resource::<FrameMaterialTableBuild>()
            .table()
            .row_count()
    }

    fn first_text_run_material_values(app: &App) -> MaterialSlotValues {
        let store = app.world().resource::<GlyphCache>().batch_store();
        let (_, batch) = store.batches().next().expect("one text batch should exist");
        let record = batch
            .run_records()
            .first()
            .expect("text batch should have one run record");
        let table = app
            .world()
            .resource::<FrameMaterialTableBuild>()
            .table()
            .rows();
        table[record.material.as_u32().to_usize()]
    }

    fn text_run_coverage_biases(app: &App) -> Vec<f32> {
        app.world()
            .resource::<GlyphCache>()
            .batch_store()
            .batches()
            .flat_map(|(_, batch)| batch.run_records().iter())
            .map(|record| record.text_coverage_bias)
            .collect()
    }

    fn layout_hash(style: &TextStyle) -> u64 {
        let mut hasher = DefaultHasher::new();
        style.hash_layout(&mut hasher);
        hasher.finish()
    }

    fn absent_text_frame_material_rows() -> usize {
        let mut app = pipeline_app();
        settle(&mut app);
        frame_material_row_count(&app)
    }

    fn drop_cached_glyph_paths(mut backend: ResMut<GlyphCache>) {
        *backend = GlyphCache::default();
    }

    fn clip_prepared_runs_outside_glyphs(mut prepared_runs: Query<&mut PreparedPanelText>) {
        for mut prepared in &mut prepared_runs {
            prepared.clip_rect = Some(NON_INTERSECTING_CLIP_RECT);
        }
    }

    #[test]
    fn unroutable_runs_append_no_material_rows() {
        let absent_rows = absent_text_frame_material_rows();

        let mut missing_glyphs = pipeline_app();
        missing_glyphs.add_systems(
            PostUpdate,
            drop_cached_glyph_paths
                .after(shaping::shape_panel_text_children)
                .before(update_panel_text_batches),
        );
        spawn_panel(&mut missing_glyphs, one_text_tree());
        settle(&mut missing_glyphs);

        assert_eq!(store_stats(&missing_glyphs), (0, 0, 0));
        assert_eq!(frame_material_row_count(&missing_glyphs), absent_rows);

        let mut fully_clipped = pipeline_app();
        fully_clipped.add_systems(
            PostUpdate,
            clip_prepared_runs_outside_glyphs
                .after(shaping::shape_panel_text_children)
                .before(update_panel_text_batches),
        );
        spawn_panel(&mut fully_clipped, one_text_tree());
        settle(&mut fully_clipped);

        assert_eq!(store_stats(&fully_clipped), (0, 0, 0));
        assert_eq!(frame_material_row_count(&fully_clipped), absent_rows);
    }

    #[test]
    fn production_pipeline_routes_panel_alpha_on_first_frame() {
        let mut app = pipeline_app();
        assert_eq!(app.world().resource::<TextAlpha>().0, AlphaMode::Blend);
        app.world_mut().spawn(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(50.0))
                .text_alpha_mode(AlphaMode::Add)
                .with_tree(one_text_tree())
                .build()
                .expect("panel should build"),
        );

        app.update();

        let store = app.world().resource::<GlyphCache>().batch_store();
        let (key, batch) = store.batches().next().expect("one text batch should exist");
        assert_eq!(batch.run_count(), 1);
        assert_eq!(
            key.pipeline_compatibility.alpha,
            AlphaMode::Add.into(),
            "the first routed batch should use the panel override"
        );
    }

    #[test]
    fn runs_route_into_one_batch_entity() {
        let mut app = pipeline_app();
        spawn_panel(&mut app, two_text_tree());
        settle(&mut app);

        let (batches, runs, glyphs) = store_stats(&app);
        assert_eq!(batches, 1, "two same-key runs share one batch");
        assert_eq!(runs, 2);
        assert_eq!(glyphs, 9, "'Alpha' + 'Beta' visible glyphs");
        assert_eq!(batch_entities(&mut app).len(), 1);
    }

    #[test]
    fn text_batch_entities_ignore_stock_mesh_picking() {
        let mut app = pipeline_app();
        spawn_panel(&mut app, one_text_tree());
        settle(&mut app);

        let entities = batch_entities(&mut app);
        assert_eq!(entities.len(), 1);
        let pickable = app
            .world()
            .get::<Pickable>(entities[0])
            .expect("text batch entity should carry Pickable::IGNORE");
        assert!(!pickable.is_hoverable);
        assert!(!pickable.should_block_lower);
    }

    #[test]
    fn batch_entity_gets_real_bounds_and_sort_translation() {
        let mut app = pipeline_app();
        spawn_panel(&mut app, two_text_tree());
        settle(&mut app);

        let entity = batch_entities(&mut app)[0];
        let aabb = *app
            .world()
            .get::<Aabb>(entity)
            .expect("batch entity should carry a hand-written Aabb");
        assert!(
            aabb.half_extents.length_squared() > 0.0,
            "the union Aabb has real extents, not the inert mesh's zeros"
        );
        assert_eq!(aabb.center, Vec3A::ZERO, "the Aabb is local-space");
        assert!(
            app.world().get::<NoAutoAabb>(entity).is_some(),
            "CalculateBounds must not replace the union Aabb"
        );
    }

    #[test]
    fn run_record_transform_matches_propagated_label_transform() {
        let mut app = pipeline_app();
        let panel = app
            .world_mut()
            .spawn((
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .with_tree(two_text_tree())
                    .build()
                    .expect("panel should build"),
                Transform::from_xyz(3.0, -2.0, 1.0),
            ))
            .id();
        settle(&mut app);

        let panel_translation = app
            .world()
            .get::<GlobalTransform>(panel)
            .expect("panel should have a GlobalTransform")
            .translation();
        assert_eq!(panel_translation, Vec3::new(3.0, -2.0, 1.0));
        let store = app.world().resource::<GlyphCache>().batch_store();
        let (_, batch) = store.batches().next().expect("one batch should exist");
        for record in batch.run_records() {
            // Labels sit at panel-local offsets, so only the propagated panel
            // translation is asserted exactly: the records' w-axis carries it.
            assert!(
                (record.transform.w_axis.z - 1.0).abs() < f32::EPSILON,
                "run record transform should carry the propagated panel z"
            );
        }
    }

    #[test]
    fn despawning_the_panel_despawns_the_emptied_batch() {
        let mut app = pipeline_app();
        let panel = spawn_panel(&mut app, two_text_tree());
        settle(&mut app);
        assert_eq!(batch_entities(&mut app).len(), 1);

        app.world_mut().entity_mut(panel).despawn();
        settle(&mut app);

        assert_eq!(store_stats(&app), (0, 0, 0));
        assert_eq!(
            batch_entities(&mut app).len(),
            0,
            "the last run leaving despawns the batch entity"
        );
    }

    #[test]
    fn hidden_panel_retains_text_material_rows_without_batched_runs() {
        let mut app = pipeline_app();
        let panel = spawn_panel(&mut app, two_text_tree());
        app.world_mut().entity_mut(panel).insert(Visibility::Hidden);
        settle(&mut app);
        let hidden_material_table = app.world().resource::<FrameMaterialTableBuild>().table();
        let hidden_material_rows = hidden_material_table.row_count();
        let hidden_material_sources = hidden_material_table.live_row_count();

        assert_eq!(
            store_stats(&app),
            (0, 0, 0),
            "a hidden panel's text should not enter the batch store"
        );
        assert!(batch_entities(&mut app).is_empty());
        assert_eq!(
            hidden_material_sources, 2,
            "hidden text should retain one live material source per prepared run",
        );

        app.world_mut()
            .entity_mut(panel)
            .insert(Visibility::Inherited);
        settle(&mut app);
        assert_eq!(
            store_stats(&app),
            (1, 2, 9),
            "restoring inherited visibility routes the existing text runs"
        );
        assert_eq!(batch_entities(&mut app).len(), 1);
        let visible_material_slots: Vec<_> = text_run_records(&app)
            .into_iter()
            .map(|record| record.material.as_u32())
            .collect();
        assert_eq!(visible_material_slots.len(), hidden_material_sources);
        assert_eq!(
            app.world()
                .resource::<FrameMaterialTableBuild>()
                .table()
                .row_count(),
            hidden_material_rows,
        );

        app.world_mut().entity_mut(panel).insert(Visibility::Hidden);
        settle(&mut app);
        assert_eq!(
            store_stats(&app),
            (0, 0, 0),
            "hiding the panel again removes its text from the batch store"
        );
        assert!(batch_entities(&mut app).is_empty());
        let hidden_again = app.world().resource::<FrameMaterialTableBuild>().table();
        assert_eq!(hidden_again.row_count(), hidden_material_rows);
        assert_eq!(hidden_again.live_row_count(), hidden_material_sources);

        app.world_mut()
            .entity_mut(panel)
            .insert(Visibility::Inherited);
        settle(&mut app);
        let reshown_material_slots: Vec<_> = text_run_records(&app)
            .into_iter()
            .map(|record| record.material.as_u32())
            .collect();
        assert_eq!(
            reshown_material_slots, visible_material_slots,
            "showing the same text again should restore the same material rows",
        );
    }

    #[test]
    fn text_edit_rewrites_records_in_the_same_batch() {
        let mut app = pipeline_app();
        let panel = spawn_panel(&mut app, two_text_tree());
        settle(&mut app);
        let entity_before = batch_entities(&mut app)[0];
        let (_, _, glyphs_before) = store_stats(&app);

        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(("Alphas", TextStyle::new(10.0)));
        builder.text(("Beta", TextStyle::new(10.0)));
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, builder.build())
                .is_ok()
        );
        settle(&mut app);

        let (batches, runs, glyphs) = store_stats(&app);
        assert_eq!(batches, 1);
        assert_eq!(runs, 2);
        assert_eq!(glyphs, glyphs_before + 1, "'Alphas' grew by one glyph");
        assert_eq!(
            batch_entities(&mut app)[0],
            entity_before,
            "a text edit reuses the batch entity"
        );
    }

    #[test]
    fn same_capacity_text_edit_stages_gpu_update_in_the_same_frame() {
        let mut app = pipeline_app();
        let panel = spawn_panel(&mut app, two_text_tree());
        settle(&mut app);
        let instance_buffer = text_instance_buffer(&app);
        let buffer_data_before = app
            .world()
            .resource::<Assets<ShaderBuffer>>()
            .get(&instance_buffer)
            .expect("the text instance buffer should exist")
            .data
            .clone();
        app.update();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(("Alphas", TextStyle::new(10.0)));
        builder.text(("Beta", TextStyle::new(10.0)));
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, builder.build())
                .is_ok()
        );
        app.world_mut().flush();
        app.update();

        assert_eq!(store_stats(&app), (1, 2, 10));
        assert_eq!(text_instance_buffer(&app), instance_buffer);
        let staged = app.world().resource::<RetainedBatchBufferUploads>();
        let staged_data = staged
            .data(&instance_buffer)
            .expect("the retained text edit should stage a same-frame GPU write");
        assert_ne!(Some(staged_data), buffer_data_before.as_deref());
    }

    #[test]
    fn alpha_cascade_change_moves_the_run_to_the_new_keys_batch() {
        let mut app = pipeline_app();
        spawn_panel(&mut app, two_text_tree());
        settle(&mut app);
        assert_eq!(store_stats(&app), (1, 2, 9));

        let label = label_entities(&mut app)[0];
        app.world_mut()
            .commands()
            .entity(label)
            .override_text_alpha(AlphaMode::Opaque);
        settle(&mut app);

        let (batches, runs, glyphs) = store_stats(&app);
        assert_eq!(
            batches, 2,
            "the alpha-changed run re-keys into its own batch"
        );
        assert_eq!(runs, 2);
        assert_eq!(glyphs, 9, "no records were lost in the move");
        assert_eq!(batch_entities(&mut app).len(), 2);
        let store = app.world().resource::<GlyphCache>().batch_store();
        let opaque_runs: usize = store
            .batches()
            .filter(|(key, _)| key.pipeline_compatibility.alpha == AlphaMode::Opaque.into())
            .map(|(_, batch)| batch.run_count())
            .sum();
        assert_eq!(
            opaque_runs, 1,
            "exactly the overridden run is keyed under the new key"
        );
    }

    #[test]
    fn text_run_inherits_panel_text_material_through_cascade() {
        let mut app = pipeline_app();
        let panel_material = material_with_metallic(&mut app, 0.61);
        app.world_mut().spawn(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(50.0))
                .text_material(panel_material)
                .with_tree(one_text_tree())
                .build()
                .expect("panel should build"),
        );
        settle(&mut app);

        assert_eq!(
            first_text_run_material_values(&app).metallic.to_bits(),
            0.61_f32.to_bits()
        );
    }

    #[test]
    fn text_run_material_wins_over_panel_text_material() {
        let mut app = pipeline_app();
        let panel_material = material_with_metallic(&mut app, 0.22);
        let run_material = material_with_metallic(&mut app, 0.77);
        app.world_mut().spawn(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(50.0))
                .text_material(panel_material)
                .with_tree(one_text_tree_with_style(
                    TextStyle::new(10.0).with_material(run_material),
                ))
                .build()
                .expect("panel should build"),
        );
        settle(&mut app);

        assert_eq!(
            first_text_run_material_values(&app).metallic.to_bits(),
            0.77_f32.to_bits()
        );
    }

    #[test]
    fn text_run_inherits_panel_hdr_text_coverage_bias_through_cascade() {
        let mut app = pipeline_app();
        app.world_mut().spawn(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(50.0))
                .hdr_text_coverage_bias(2.0)
                .with_tree(one_text_tree())
                .build()
                .expect("panel should build"),
        );
        settle(&mut app);

        assert_eq!(text_run_coverage_biases(&app), vec![2.0]);
    }

    #[test]
    fn text_run_hdr_text_coverage_bias_wins_over_panel_default() {
        let mut app = pipeline_app();
        app.world_mut().spawn(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(50.0))
                .hdr_text_coverage_bias(2.0)
                .with_tree(one_text_tree_with_style(
                    TextStyle::new(10.0).with_hdr_text_coverage_bias(-1.25),
                ))
                .build()
                .expect("panel should build"),
        );
        settle(&mut app);

        assert_eq!(text_run_coverage_biases(&app), vec![-1.25]);
    }

    #[test]
    fn global_hdr_text_coverage_bias_refreshes_run_records_without_splitting_batch() {
        let mut app = pipeline_app();
        spawn_panel(&mut app, two_text_tree());
        settle(&mut app);
        let entity_before = batch_entities(&mut app)[0];
        assert_eq!(store_stats(&app), (1, 2, 9));
        assert_eq!(text_run_coverage_biases(&app), vec![0.0, 0.0]);

        app.world_mut().resource_mut::<HdrTextCoverageBias>().0 = 2.0;
        settle(&mut app);

        assert_eq!(store_stats(&app), (1, 2, 9));
        assert_eq!(batch_entities(&mut app), vec![entity_before]);
        assert_eq!(text_run_coverage_biases(&app), vec![2.0, 2.0]);
    }

    #[test]
    fn scalar_distinct_text_materials_share_one_batch_with_distinct_rows() {
        let mut app = pipeline_app();
        let first_material = material_with_metallic(&mut app, 0.21);
        let second_material = material_with_metallic(&mut app, 0.84);
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(("Alpha", TextStyle::new(10.0).with_material(first_material)));
        builder.text(("Beta", TextStyle::new(10.0).with_material(second_material)));
        spawn_panel(&mut app, builder.build());
        settle(&mut app);

        let (batches, runs, glyphs) = store_stats(&app);
        assert_eq!(batches, 1, "scalar/vector-only differences stay batched");
        assert_eq!(runs, 2);
        assert_eq!(glyphs, 9);
        let store = app.world().resource::<GlyphCache>().batch_store();
        let (_, batch) = store.batches().next().expect("one text batch should exist");
        let table = app
            .world()
            .resource::<FrameMaterialTableBuild>()
            .table()
            .rows();
        let metallic_values: Vec<u32> = batch
            .run_records()
            .iter()
            .map(|record| {
                table[record.material.as_u32().to_usize()]
                    .metallic
                    .to_bits()
            })
            .collect();
        assert_eq!(
            metallic_values,
            vec![0.21_f32.to_bits(), 0.84_f32.to_bits()],
            "each run keeps its own frame material row"
        );
    }

    #[test]
    fn same_texture_text_materials_share_texture_batch_and_write_box_uvs() {
        let mut app = pipeline_app();
        let texture = Handle::<Image>::default();
        let material = material_with_texture(&mut app, texture);
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(("AB", TextStyle::new(10.0).with_material(material.clone())));
        builder.text(("CD", TextStyle::new(10.0).with_material(material)));
        let panel = spawn_panel(&mut app, builder.build());
        settle(&mut app);

        let (batches, runs, glyphs) = store_stats(&app);
        assert_eq!(
            batches, 1,
            "same texture resource stays in one texture batch"
        );
        assert_eq!(runs, 2);
        assert_eq!(glyphs, 4);
        let store = app.world().resource::<GlyphCache>().batch_store();
        let (key, batch) = store
            .batches()
            .next()
            .expect("one texture batch should exist");
        assert!(
            key.resource_compatibility.base_color_texture.is_some(),
            "the texture resource remains a batch-key splitter"
        );
        let before: Vec<(Vec2, Vec2)> = batch
            .path_records()
            .iter()
            .map(|record| (record.box_uv_min, record.box_uv_size))
            .collect();
        assert!(
            before
                .windows(2)
                .any(|pair| pair[0].0.x.to_bits() != pair[1].0.x.to_bits()),
            "glyph records sample distinct horizontal regions of the run box"
        );
        assert!(
            before.iter().all(|(uv_min, uv_size)| {
                uv_min.x >= 0.0
                    && uv_min.y >= 0.0
                    && uv_min.x + uv_size.x <= 1.0 + f32::EPSILON
                    && uv_min.y + uv_size.y <= 1.0 + f32::EPSILON
                    && uv_size.x > 0.0
                    && uv_size.y > 0.0
            }),
            "box UVs stay inside the run-local 0..1 box"
        );

        app.world_mut().entity_mut(panel).insert(Visibility::Hidden);
        settle(&mut app);
        app.world_mut()
            .entity_mut(panel)
            .insert(Visibility::Inherited);
        settle(&mut app);
        let store = app.world().resource::<GlyphCache>().batch_store();
        let (_, batch) = store.batches().next().expect("texture batch should return");
        let after: Vec<(Vec2, Vec2)> = batch
            .path_records()
            .iter()
            .map(|record| (record.box_uv_min, record.box_uv_size))
            .collect();
        assert_eq!(after, before, "box UVs are stable across hide/reroute");
    }

    #[test]
    fn reused_text_run_material_swap_updates_resolved_material_without_layout_cache_change() {
        let mut app = pipeline_app();
        let first_material = material_with_metallic(&mut app, 0.31);
        let second_material = material_with_metallic(&mut app, 0.82);
        let first_style = TextStyle::new(10.0).with_material(first_material);
        let second_style = TextStyle::new(10.0).with_material(second_material.clone());
        let panel = spawn_panel(&mut app, one_text_tree_with_style(first_style.clone()));
        settle(&mut app);
        let label_before = label_entities(&mut app)[0];
        assert_eq!(
            first_text_run_material_values(&app).metallic.to_bits(),
            0.31_f32.to_bits()
        );

        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, one_text_tree_with_style(second_style.clone()))
                .is_ok()
        );
        settle(&mut app);
        let label_after = label_entities(&mut app)[0];
        let resolved = app
            .world()
            .get::<Resolved<TextMaterial>>(label_after)
            .expect("text run should carry Resolved<TextMaterial>");

        assert_eq!(
            label_after, label_before,
            "the text run entity should be reused"
        );
        assert_eq!(resolved.0.0, second_material);
        assert_eq!(
            first_text_run_material_values(&app).metallic.to_bits(),
            0.82_f32.to_bits()
        );
        assert_eq!(first_style.as_measure(), second_style.as_measure());
        assert_eq!(layout_hash(&first_style), layout_hash(&second_style));
    }

    /// Per-batch (`depth_bias`, OIT depth offset) read off the single live
    /// batch's material asset.
    fn batch_material_values(app: &App) -> (f32, f32) {
        let store = app.world().resource::<GlyphCache>().batch_store();
        let (_, batch) = store.batches().next().expect("one batch should exist");
        let gpu = batch.gpu.as_ref().expect("batch should have GPU assets");
        let material = app
            .world()
            .resource::<Assets<PathExtendedMaterial>>()
            .get(&gpu.material)
            .expect("batch material asset should exist");
        (
            material.base.depth_bias,
            text::text_material_oit_depth_offset(material),
        )
    }

    fn batch_z_indices(app: &App) -> Vec<DrawZIndex> {
        let store = app.world().resource::<GlyphCache>().batch_store();
        let mut z_indices: Vec<DrawZIndex> = store.batches().map(|(key, _)| key.z_index).collect();
        z_indices.sort_unstable();
        z_indices
    }

    fn batch_material_depth_biases(app: &App) -> Vec<(DrawZIndex, u32)> {
        let store = app.world().resource::<GlyphCache>().batch_store();
        let materials = app.world().resource::<Assets<PathExtendedMaterial>>();
        let mut depth_biases: Vec<(DrawZIndex, u32)> = store
            .batches()
            .map(|(key, batch)| {
                let gpu = batch
                    .gpu
                    .as_ref()
                    .expect("text batch should have GPU assets");
                let material = materials
                    .get(&gpu.material)
                    .expect("text batch material asset should exist");
                (key.z_index, material.base.depth_bias.to_bits())
            })
            .collect();
        depth_biases.sort_by_key(|(z_index, _)| *z_index);
        depth_biases
    }

    fn panel_text_z_indices(app: &mut App) -> Vec<DrawZIndex> {
        let mut state = app.world_mut().query::<&PanelTextDrawZIndex>();
        let mut z_indices: Vec<DrawZIndex> =
            state.iter(app.world()).map(|z_index| z_index.0).collect();
        z_indices.sort_unstable();
        z_indices
    }

    #[test]
    fn default_text_across_panels_shares_one_batch() {
        let mut app = pipeline_app();
        spawn_panel(&mut app, two_text_tree());
        spawn_panel(&mut app, two_text_tree());
        settle(&mut app);

        assert_eq!(store_stats(&app), (1, 4, 18));
        assert_eq!(batch_z_indices(&app), vec![DrawZIndex::default()]);
        assert_eq!(batch_entities(&mut app).len(), 1);
    }

    #[test]
    fn z_index_change_moves_run_to_level_batch() {
        let mut app = pipeline_app();
        spawn_panel(&mut app, two_text_tree());
        settle(&mut app);
        assert_eq!(store_stats(&app), (1, 2, 9));

        let label = label_entities(&mut app)[0];
        app.world_mut()
            .commands()
            .entity(label)
            .insert(PanelTextDrawZIndex(1.into()));
        settle(&mut app);

        assert_eq!(store_stats(&app), (2, 2, 9));
        assert_eq!(
            batch_z_indices(&app),
            vec![DrawZIndex::default(), DrawZIndex(1)]
        );
        assert_eq!(batch_entities(&mut app).len(), 2);
    }

    #[test]
    fn text_element_z_index_authors_z_index_batches() {
        let mut app = pipeline_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(
            Text::new("Lower", TextStyle::new(10.0)).layout(El::new().z_index(LOWERED_LEVEL)),
        );
        builder
            .text(Text::new("Raise", TextStyle::new(10.0)).layout(El::new().z_index(RAISED_LEVEL)));
        spawn_panel(&mut app, builder.build());
        settle(&mut app);

        assert_eq!(
            panel_text_z_indices(&mut app),
            vec![LOWERED_LEVEL, RAISED_LEVEL]
        );
        assert_eq!(batch_z_indices(&app), vec![LOWERED_LEVEL, RAISED_LEVEL]);
        let (batches, runs, _) = store_stats(&app);
        assert_eq!(batches, 2);
        assert_eq!(runs, 2);

        let lowered_depth_bias = 0.0_f32;
        let raised_depth_bias = constants::LAYER_DEPTH_BIAS;
        assert_eq!(
            batch_material_depth_biases(&app),
            vec![
                (LOWERED_LEVEL, lowered_depth_bias.to_bits()),
                (RAISED_LEVEL, raised_depth_bias.to_bits()),
            ],
        );
        assert!(lowered_depth_bias < raised_depth_bias);
    }

    #[test]
    fn default_text_batch_material_uses_z_index_rank() {
        let mut app = pipeline_app();
        spawn_panel(&mut app, two_text_tree());
        settle(&mut app);

        let (depth_bias, oit_depth_offset) = batch_material_values(&app);
        assert_eq!(depth_bias.to_bits(), 0.0_f32.to_bits());
        assert_eq!(oit_depth_offset.to_bits(), 0.0f32.to_bits());
    }

    fn run_record_depths(app: &App) -> Vec<(f32, f32)> {
        let store = app.world().resource::<GlyphCache>().batch_store();
        let mut records: Vec<_> = store
            .batches()
            .flat_map(|(_, batch)| batch.run_records())
            .map(|record| (record.clip_depth_nudge, record.oit_depth_offset))
            .collect();
        records.sort_by(|left, right| left.0.total_cmp(&right.0));
        records
    }

    #[test]
    fn text_run_records_use_command_ordinals() {
        let mut app = pipeline_app();
        spawn_panel(&mut app, two_text_tree());
        settle(&mut app);

        assert_eq!(
            run_record_depths(&app),
            vec![
                (0.0, constants::OIT_DEPTH_STEP),
                (1.0, 2.0 * constants::OIT_DEPTH_STEP),
            ],
        );
    }

    /// Mirrors `text_alpha`'s `apply_state_and_rebuild_hud`: one frame both
    /// changes the global alpha default and despawns + respawns a panel whose
    /// alpha is pinned by a panel-level override. The respawned runs must key
    /// by the pin, not by the default that was active while they routed.
    #[test]
    fn respawned_pinned_panel_keys_by_its_override_not_the_default() {
        fn spawn_pinned_panel(app: &mut App) -> Entity {
            app.world_mut()
                .spawn(
                    DiegeticPanel::world()
                        .size(Mm(100.0), Mm(50.0))
                        .text_alpha_mode(AlphaMode::Blend)
                        .with_tree(two_text_tree())
                        .build()
                        .expect("panel should build"),
                )
                .id()
        }

        let mut app = pipeline_app();
        let panel = spawn_pinned_panel(&mut app);
        settle(&mut app);
        assert_eq!(store_stats(&app), (1, 2, 9));

        // The live sequence: default change + rebuild in the same frame.
        app.world_mut().resource_mut::<TextAlpha>().0 = AlphaMode::Multiply;
        app.world_mut().entity_mut(panel).despawn();
        spawn_pinned_panel(&mut app);
        settle(&mut app);

        let (batches, runs, glyphs) = store_stats(&app);
        assert_eq!(runs, 2, "the respawned panel's runs are routed");
        assert_eq!(glyphs, 9);
        let store = app.world().resource::<GlyphCache>().batch_store();
        let keys: Vec<AlphaMode> = store
            .batches()
            .map(|(key, _)| key.pipeline_compatibility.alpha.into())
            .collect();
        assert_eq!(
            (batches, keys.as_slice()),
            (1, &[AlphaMode::Blend][..]),
            "pinned runs key by the panel override, not the changed default"
        );
    }

    #[test]
    fn fill_color_edit_stays_in_batch_as_a_material_row_write() {
        let mut app = pipeline_app();
        let panel = spawn_panel(&mut app, two_text_tree());
        settle(&mut app);
        let entity_before = batch_entities(&mut app)[0];
        let path_records_before: Vec<PathQuadRecord> = {
            let store = app.world().resource::<GlyphCache>().batch_store();
            let (_, batch) = store.batches().next().expect("one batch should exist");
            batch.path_records().to_vec()
        };

        // Same text, new color: a material-table row write, not a re-key.
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text((
            "Alpha",
            TextStyle::new(10.0).with_color(Color::srgb(1.0, 0.0, 0.0)),
        ));
        builder.text(("Beta", TextStyle::new(10.0)));
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, builder.build())
                .is_ok()
        );
        settle(&mut app);

        assert_eq!(
            store_stats(&app),
            (1, 2, 9),
            "one batch survives the recolor"
        );
        assert_eq!(
            batch_entities(&mut app)[0],
            entity_before,
            "a fill-color edit reuses the batch entity"
        );
        let store = app.world().resource::<GlyphCache>().batch_store();
        let (_, batch) = store.batches().next().expect("one batch should exist");
        assert_eq!(
            batch.path_records(),
            path_records_before.as_slice(),
            "recoloring must not rewrite path-quad records"
        );
        let material_slots: Vec<u32> = batch
            .run_records()
            .iter()
            .map(|record| record.material.as_u32())
            .collect();
        let table = app
            .world()
            .resource::<FrameMaterialTableBuild>()
            .table()
            .rows();
        let red = Color::srgb(1.0, 0.0, 0.0).to_linear();
        assert!(
            material_slots.iter().any(|slot| {
                table.get(slot.to_usize()).is_some_and(|row| {
                    row.base_color == Vec4::new(red.red, red.green, red.blue, red.alpha)
                })
            }),
            "the recolored run's current material table row carries the new fill color"
        );
    }

    #[test]
    fn text_drops_normal_and_depth_maps_it_cannot_sample() {
        // Glyph quads have UVs but no tangents, so normal and parallax (depth)
        // maps would sample an undefined basis; UV-sampled maps survive.
        let authored = StandardMaterial {
            base_color_texture: Some(Handle::default()),
            normal_map_texture: Some(Handle::default()),
            depth_map: Some(Handle::default()),
            ..Default::default()
        };

        let resolved = strip_tangent_dependent_maps(&authored);

        assert!(resolved.normal_map_texture.is_none());
        assert!(resolved.depth_map.is_none());
        assert!(
            resolved.base_color_texture.is_some(),
            "uv-sampled textures survive the strip"
        );
    }

    #[test]
    fn shadow_mode_change_rekeys_into_a_not_shadow_caster_batch() {
        let mut app = pipeline_app();
        let panel = spawn_panel(&mut app, two_text_tree());
        settle(&mut app);
        assert_eq!(store_stats(&app).0, 1);

        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text((
            "Alpha",
            TextStyle::new(10.0).with_shadow_mode(GlyphShadowMode::None),
        ));
        builder.text(("Beta", TextStyle::new(10.0)));
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, builder.build())
                .is_ok()
        );
        settle(&mut app);

        let (batches, runs, _) = store_stats(&app);
        assert_eq!(batches, 2, "the shadow-mode run re-keys into its own batch");
        assert_eq!(runs, 2);
        let entities = batch_entities(&mut app);
        let non_casters = entities
            .iter()
            .filter(|&&entity| app.world().get::<NotShadowCaster>(entity).is_some())
            .count();
        assert_eq!(
            (non_casters, entities.len() - non_casters),
            (1, 1),
            "the GlyphShadowMode::None batch entity opts out of shadow casting"
        );
    }

    #[test]
    fn same_frame_cascade_move_and_despawn_leave_the_store_consistent() {
        let mut app = pipeline_app();
        let panel = spawn_panel(&mut app, two_text_tree());
        settle(&mut app);
        let label = label_entities(&mut app)[0];

        // Same frame: the run's key changes AND its panel despawns. The
        // routing system must observe one consistent end state (decision-4/9
        // order independence).
        app.world_mut()
            .commands()
            .entity(label)
            .override_text_alpha(AlphaMode::Opaque);
        app.world_mut().commands().entity(panel).despawn();
        settle(&mut app);

        assert_eq!(store_stats(&app), (0, 0, 0));
        assert!(batch_entities(&mut app).is_empty());
        assert!(
            !app.world()
                .resource::<GlyphCache>()
                .batch_store()
                .is_routed(RunStorageKey::from(label)),
            "the despawned run left no membership behind"
        );
    }

    /// The `PathBatchResources` doc contract: every commit payload is padded to the
    /// buffer's capacity, so its byte length never changes between growths —
    /// a constant-length `set_data` writes the existing wgpu buffer in place
    /// and live material bind groups keep reading it. A refactor that drops
    /// the padding fails here instead of at a parity screenshot.
    #[test]
    fn commit_payloads_keep_a_constant_length_between_growths() {
        let glyph = PathQuadRecord {
            rect_min:          Vec2::ZERO,
            rect_size:         Vec2::ONE,
            uv_min:            Vec2::ZERO,
            uv_size:           Vec2::ONE,
            box_uv_min:        Vec2::ZERO,
            box_uv_size:       Vec2::ONE,
            packed_path_index: 0,
            render_index:      0,
            box_uv_flip_x:     0,
        };
        let record = PathRenderRecord {
            transform:          Mat4::IDENTITY,
            material:           SdfPaintMaterial::NotAuthored.to_gpu(),
            render_mode:        1,
            clip_depth_nudge:   0.0,
            oit_depth_offset:   0.0,
            aa_flags:           AntiAlias::Both.aa_flags(),
            text_coverage_bias: 0.0,
            text_em_height:     0.0,
        };

        for count in 0..=8_usize {
            assert_eq!(
                padded_glyph_records(&vec![glyph; count], 8).len(),
                8,
                "glyph payload length must equal capacity at {count} records"
            );
            assert_eq!(
                padded_run_records(&vec![record; count], 8).len(),
                8,
                "run payload length must equal capacity at {count} records"
            );
        }
    }

    // ── Widget visual-slot overrides ────────────────────────────────────────

    fn widget_pipeline_app() -> App {
        let mut app = pipeline_app();
        app.add_plugins(WidgetsPlugin);
        app
    }

    fn slotted_text_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("styled"), |builder| {
            builder.text(
                Text::new("Alpha", TextStyle::new(10.0)).layout(El::new().visual_slot(TEXT_SLOT)),
            );
        });
        builder.text(("Beta", TextStyle::new(10.0)));
        builder.build()
    }

    fn hovered_caption_tree(appearance: Appearance) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new().background(Color::WHITE).button("styled"),
            |builder| {
                builder.text(
                    Text::new("Alpha", TextStyle::new(10.0))
                        .layout(builder.child(El::new()).hovered(appearance)),
                );
            },
        );
        builder.build()
    }

    fn hovered_text_surface_tree(appearance: Appearance) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("styled"), |builder| {
            builder.text(
                Text::new("Alpha", TextStyle::new(10.0)).layout(
                    builder
                        .child(
                            El::new()
                                .background(Color::WHITE)
                                .border(Border::all(1.0, Color::BLACK)),
                        )
                        .hovered(appearance),
                ),
            );
        });
        builder.build()
    }

    fn text_recipient_index(app: &App, widget: Entity) -> usize {
        app.world()
            .get::<WidgetVisualSlots>(widget)
            .expect("widget should carry visual slots")
            .elements()
            .iter()
            .find(|(_, capabilities)| capabilities.contains(VisualElementCapabilities::TEXT))
            .map(|(element_index, _)| *element_index)
            .expect("widget should retain a text recipient")
    }

    fn linear_base_color(color: Color) -> Vec4 {
        let linear = color.to_linear();
        Vec4::new(linear.red, linear.green, linear.blue, linear.alpha)
    }

    fn styled_widget(app: &mut App) -> Entity {
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<PanelWidget>>();
        let widgets: Vec<Entity> = query.iter(app.world()).collect();
        assert_eq!(widgets.len(), 1, "expected exactly one reified widget");
        widgets[0]
    }

    fn set_text_slot_override(app: &mut App, widget: Entity, value: VisualSlotOverride) {
        let mut overrides = WidgetVisualOverrides::default();
        overrides.set(TEXT_SLOT, value);
        app.world_mut().entity_mut(widget).insert(overrides);
    }

    fn run_material_base_colors(app: &App) -> Vec<Vec4> {
        let table = app
            .world()
            .resource::<FrameMaterialTableBuild>()
            .table()
            .rows();
        app.world()
            .resource::<GlyphCache>()
            .batch_store()
            .batches()
            .flat_map(|(_, batch)| batch.run_records().to_vec())
            .map(|record| table[record.material.as_u32().to_usize()].base_color)
            .collect()
    }

    #[test]
    fn hovered_button_caption_brightens_without_a_fill_override() {
        let mut app = widget_pipeline_app();
        let panel = spawn_panel(
            &mut app,
            hovered_caption_tree(Appearance::new().text_color(TEXT_OVERRIDE_COLOR)),
        );
        settle(&mut app);
        let authored_colors = run_material_base_colors(&app);
        assert_eq!(authored_colors.len(), 1);
        let widget = styled_widget(&mut app);
        let text_index = text_recipient_index(&app, widget);
        let fill_index = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .expect("widget should carry visual slots")
            .elements()
            .iter()
            .find(|(_, capabilities)| capabilities.contains(VisualElementCapabilities::SDF_FILL))
            .map(|(element_index, _)| *element_index)
            .expect("button root should retain its authored fill");

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(
            run_material_base_colors(&app),
            vec![linear_base_color(TEXT_OVERRIDE_COLOR)]
        );
        let index = app.world().resource::<VisualOverrideIndex>();
        assert_eq!(index.get(panel, fill_index), None);
        assert_eq!(
            index
                .get(panel, text_index)
                .and_then(|override_value| override_value.fill_color),
            None
        );
    }

    #[test]
    fn text_state_color_leaves_its_authored_background_and_border_unchanged() {
        let mut app = widget_pipeline_app();
        let panel = spawn_panel(
            &mut app,
            hovered_text_surface_tree(Appearance::new().text_color(TEXT_OVERRIDE_COLOR)),
        );
        settle(&mut app);
        let authored_colors = run_material_base_colors(&app);
        assert_eq!(authored_colors.len(), 1);
        let widget = styled_widget(&mut app);
        let text_index = text_recipient_index(&app, widget);
        let capabilities = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .expect("widget should carry visual slots")
            .elements()
            .iter()
            .find(|(element_index, _)| *element_index == text_index)
            .map(|(_, capabilities)| *capabilities)
            .expect("text surface should retain its visual recipient");
        assert!(capabilities.contains(VisualElementCapabilities::SDF_FILL));
        assert!(capabilities.contains(VisualElementCapabilities::SDF_BORDER));

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(
            run_material_base_colors(&app),
            vec![linear_base_color(TEXT_OVERRIDE_COLOR)]
        );
        let override_value = app
            .world()
            .resource::<VisualOverrideIndex>()
            .get(panel, text_index)
            .expect("hovered text surface should receive an override");
        assert_eq!(override_value.text_color, Some(TEXT_OVERRIDE_COLOR));
        assert_eq!(override_value.fill_color, None);
        assert_eq!(override_value.border_color, None);
    }

    #[test]
    fn state_material_overrides_panel_text_material_and_restores_it() {
        let mut app = widget_pipeline_app();
        let panel_material = material_with_metallic(&mut app, 0.22);
        let state_material = material_with_metallic(&mut app, 0.77);
        let tree = hovered_caption_tree(Appearance::new().material(state_material));
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .text_material(panel_material)
                    .with_tree(tree)
                    .build()
                    .expect("panel should build"),
            )
            .id();
        settle(&mut app);
        assert_eq!(
            first_text_run_material_values(&app).metallic.to_bits(),
            0.22_f32.to_bits()
        );

        let widget = styled_widget(&mut app);
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert_eq!(
            first_text_run_material_values(&app).metallic.to_bits(),
            0.77_f32.to_bits()
        );

        app.world_mut()
            .entity_mut(widget)
            .remove::<PickingInteraction>();
        app.update();
        assert_eq!(
            first_text_run_material_values(&app).metallic.to_bits(),
            0.22_f32.to_bits()
        );
        assert!(app.world().get_entity(panel).is_ok());
    }

    #[test]
    fn text_color_override_patches_material_row_without_rerouting() {
        let mut app = widget_pipeline_app();
        spawn_panel(&mut app, slotted_text_tree());
        settle(&mut app);
        let entities_before = batch_entities(&mut app);
        assert_eq!(entities_before.len(), 1);
        let (batches, runs, _) = store_stats(&app);
        assert_eq!((batches, runs), (1, 2));
        let colors_before = run_material_base_colors(&app);
        assert_eq!(colors_before[0], colors_before[1]);
        let authored_color = colors_before[0];

        let widget = styled_widget(&mut app);
        set_text_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_text_color(TEXT_OVERRIDE_COLOR),
        );
        app.update();

        let colors_after = run_material_base_colors(&app);
        let override_linear = TEXT_OVERRIDE_COLOR.to_linear();
        let override_color = Vec4::new(
            override_linear.red,
            override_linear.green,
            override_linear.blue,
            override_linear.alpha,
        );
        assert_eq!(
            colors_after
                .iter()
                .filter(|color| **color == override_color)
                .count(),
            1,
            "exactly the slotted run's material row should recolor",
        );
        assert_eq!(
            colors_after
                .iter()
                .filter(|color| **color == authored_color)
                .count(),
            1,
            "the unslotted run's material row stays authored",
        );
        assert_eq!(store_stats(&app).0, 1, "a color change must not re-route");
        assert_eq!(batch_entities(&mut app), entities_before);

        app.world_mut()
            .get_mut::<WidgetVisualOverrides>(widget)
            .expect("widget should keep its override component")
            .clear(TEXT_SLOT);
        app.update();
        assert_eq!(
            run_material_base_colors(&app),
            vec![authored_color, authored_color],
        );
    }

    #[test]
    fn path_color_override_leaves_text_material_rows_authored() {
        let mut app = widget_pipeline_app();
        spawn_panel(&mut app, slotted_text_tree());
        settle(&mut app);
        let authored_colors = run_material_base_colors(&app);
        let widget = styled_widget(&mut app);

        set_text_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_path_color(TEXT_OVERRIDE_COLOR),
        );
        app.update();

        assert_eq!(run_material_base_colors(&app), authored_colors);
    }

    #[test]
    fn text_material_override_rekeys_run_and_retires_the_emptied_batch() {
        let mut app = widget_pipeline_app();
        let textured_material = material_with_texture(&mut app, Handle::default());
        spawn_panel(&mut app, slotted_text_tree());
        settle(&mut app);
        let entities_before = batch_entities(&mut app);
        assert_eq!(entities_before.len(), 1);

        let widget = styled_widget(&mut app);
        set_text_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_material(textured_material),
        );
        app.update();

        let entities_after = batch_entities(&mut app);
        assert_eq!(entities_after.len(), 2);
        assert_eq!(store_stats(&app), (2, 2, store_stats(&app).2));
        let destination = entities_after
            .iter()
            .copied()
            .find(|entity| !entities_before.contains(entity))
            .expect("the re-keyed run should spawn a destination batch");
        let pickable = app
            .world()
            .get::<Pickable>(destination)
            .expect("destination batch entity should carry Pickable::IGNORE");
        assert!(!pickable.is_hoverable);
        assert!(!pickable.should_block_lower);

        app.world_mut()
            .get_mut::<WidgetVisualOverrides>(widget)
            .expect("widget should keep its override component")
            .clear(TEXT_SLOT);
        app.update();
        assert_eq!(batch_entities(&mut app), entities_before);
        assert!(
            app.world().get_entity(destination).is_err(),
            "the emptied destination batch should be retired",
        );
    }

    fn spawn_rotated_panel(app: &mut App, tree: LayoutTree, rotation: Quat) -> Entity {
        app.world_mut()
            .spawn((
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .with_tree(tree)
                    .build()
                    .expect("panel should build"),
                Transform::from_rotation(rotation),
            ))
            .id()
    }

    fn text_run_records(app: &App) -> Vec<PathRenderRecord> {
        app.world()
            .resource::<GlyphCache>()
            .batch_store()
            .batches()
            .flat_map(|(_, batch)| batch.run_records().to_vec())
            .collect()
    }

    fn text_upload_counts(app: &App) -> (usize, usize) {
        let perf = app.world().resource::<DiegeticPerfStats>();
        (
            perf.batch.instance_uploads.get(),
            perf.batch.run_table_uploads.get(),
        )
    }

    struct TextOverrideAuthoritySnapshot {
        labels:        Vec<(Entity, Transform)>,
        widget:        Transform,
        panel_tick:    Tick,
        computed_tick: Tick,
    }

    fn text_override_authority_snapshot(
        app: &App,
        panel: Entity,
        widget: Entity,
        labels: &[Entity],
    ) -> TextOverrideAuthoritySnapshot {
        TextOverrideAuthoritySnapshot {
            labels:        labels
                .iter()
                .map(|&label| {
                    (
                        label,
                        *app.world()
                            .get::<Transform>(label)
                            .expect("label should carry a transform"),
                    )
                })
                .collect(),
            widget:        *app
                .world()
                .get::<Transform>(widget)
                .expect("widget should carry a transform"),
            panel_tick:    app
                .world()
                .entity(panel)
                .get_ref::<DiegeticPanel>()
                .expect("panel role should be live")
                .last_changed(),
            computed_tick: app
                .world()
                .entity(panel)
                .get_ref::<ComputedDiegeticPanel>()
                .expect("panel should have computed output")
                .last_changed(),
        }
    }

    fn assert_text_override_authority_unchanged(
        app: &App,
        panel: Entity,
        widget: Entity,
        snapshot: &TextOverrideAuthoritySnapshot,
    ) {
        for (label, transform) in &snapshot.labels {
            assert_eq!(
                app.world().get::<Transform>(*label),
                Some(transform),
                "an override must not move a text-run entity transform",
            );
        }
        assert_eq!(app.world().get::<Transform>(widget), Some(&snapshot.widget));
        assert_eq!(
            app.world()
                .entity(panel)
                .get_ref::<DiegeticPanel>()
                .expect("panel role should be live")
                .last_changed(),
            snapshot.panel_tick,
            "an override must not fire Changed<DiegeticPanel>",
        );
        assert_eq!(
            app.world()
                .entity(panel)
                .get_ref::<ComputedDiegeticPanel>()
                .expect("panel should have computed output")
                .last_changed(),
            snapshot.computed_tick,
            "an override must not fire Changed<ComputedDiegeticPanel>",
        );
    }

    #[test]
    fn text_offset_override_moves_only_the_slotted_run_in_panel_space() {
        let mut app = widget_pipeline_app();
        let panel = spawn_rotated_panel(
            &mut app,
            slotted_text_tree(),
            Quat::from_rotation_z(FRAC_PI_2),
        );
        settle(&mut app);
        assert_eq!(
            text_upload_counts(&app),
            (0, 0),
            "settled pipeline should be quiet"
        );
        let records_before = text_run_records(&app);
        assert_eq!(records_before.len(), 2);
        let labels = label_entities(&mut app);
        let widget = styled_widget(&mut app);
        let authority = text_override_authority_snapshot(&app, panel, widget, &labels);

        set_text_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_offset(TEXT_SLOT_OFFSET),
        );
        app.update();

        // The offset is panel-local: the rotated panel's global transform
        // maps it to the world-space delta the record matrix must carry.
        let panel_global = *app
            .world()
            .get::<GlobalTransform>(panel)
            .expect("panel should carry a global transform");
        let world_delta = panel_global
            .affine()
            .transform_vector3(TEXT_SLOT_OFFSET.extend(0.0))
            .extend(0.0);
        let records_after = text_run_records(&app);
        assert_eq!(records_after.len(), 2);
        let moved: Vec<usize> = records_before
            .iter()
            .zip(&records_after)
            .enumerate()
            .filter(|(_, (before, after))| before.transform != after.transform)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(moved.len(), 1, "exactly the slotted run's record moves");
        let mut expected_transform = records_before[moved[0]].transform;
        expected_transform.w_axis += world_delta;
        assert_eq!(records_after[moved[0]].transform, expected_transform);
        assert_text_override_authority_unchanged(&app, panel, widget, &authority);

        // Author the identical override a second time: the routes rebuild
        // equal records, so no record changes and no GPU buffer re-uploads.
        set_text_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_offset(TEXT_SLOT_OFFSET),
        );
        app.update();
        assert_eq!(
            text_upload_counts(&app),
            (0, 0),
            "a repeated identical override is a no-op",
        );
        assert_eq!(text_run_records(&app), records_after);

        app.world_mut()
            .get_mut::<WidgetVisualOverrides>(widget)
            .expect("widget should keep its override component")
            .clear(TEXT_SLOT);
        app.update();
        assert_eq!(
            text_run_records(&app),
            records_before,
            "clearing the override restores the authored record transforms",
        );
    }
}
