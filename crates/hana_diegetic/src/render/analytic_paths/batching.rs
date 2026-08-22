//! Text-run batch store for analytic path instancing: groups runs by batch key,
//! owns each batch's CPU record vectors and GPU handles, and derives run ranges
//! by rebuild.
//!
//! Membership has a single mutation point — [`TextRunBatchStore::upsert_run`] /
//! [`TextRunBatchStore::remove_run`] update the run→batch index and the batch's
//! run set together — and ranges have a single writer: `rebuild` recomputes
//! them from the live run set, so they cannot go stale relative to the record
//! vectors they index. The store is plain data; the routing systems own all
//! entity and asset work (spawning batch entities, creating meshes and
//! buffers, uploading dirty records).

use std::ops::Range;

use bevy::color::Color;
use bevy::math::Mat4;
use bevy::math::Vec2;
use bevy::math::Vec3;
use bevy::math::Vec4;
use bevy::math::Vec4Swizzles;
use bevy::pbr::StandardMaterial;
use bevy::prelude::AlphaMode;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::Mesh;
use bevy::render::storage::ShaderBuffer;
use bevy_kana::ToU32;
use bevy_kana::ToUsize;

use super::material::PathExtendedMaterial;
use super::packing::PathQuadRecord;
use super::packing::PathRenderRecord;
use crate::DrawZIndex;
use crate::layout::DrawBatchFamily;
use crate::layout::Lighting;
use crate::layout::Sidedness;
use crate::render;
use crate::render::BatchRenderLayers;
use crate::render::Dirty;
use crate::render::VisualShadow;
use crate::render::batch_key::PipelineCompatibility;
use crate::render::batch_key::ResourceCompatibility;
use crate::render::batch_store::Batch;
use crate::render::batch_store::BatchEntry;
use crate::render::batch_store::BatchStore;
use crate::render::draw_order::DrawOrderIndex;
use crate::render::draw_order::DrawZIndexRank;
use crate::render::material_table::GpuMaterialSlotId;
use crate::render::material_table::MaterialSlotCandidate;
use crate::render::material_table::MaterialSlotId;
use crate::text::RunStorageKey;

/// Map key for one `PathBatchResources` entry in an analytic-path batch store.
///
/// Scalar/vector PBR values are deliberately absent: a `PathRenderRecord`
/// carries the current frame's material table row, while this key carries only
/// sort, pass, pipeline, and resource facts that must agree inside one draw.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PathBatchKey {
    /// Authored z-index splitter for this batch's command records.
    pub z_index:                DrawZIndex,
    /// Dense panel-local rank for `z_index`, used by the batch material
    /// `StandardMaterial::depth_bias`.
    pub z_index_rank:           DrawZIndexRank,
    /// Renderer family that owns this analytic path batch.
    pub batch_family:           DrawBatchFamily,
    /// Shadow participation for this analytic path draw.
    pub shadow:                 VisualShadow,
    /// Render layers copied from the owning panel or text run scope.
    pub layers:                 BatchRenderLayers,
    /// Material-derived pipeline facts that must agree inside this path draw.
    pub pipeline_compatibility: PipelineCompatibility,
    /// Texture and bind-group facts copied into the path render material.
    pub resource_compatibility: ResourceCompatibility,
}

/// Projects an analytic producer's effective source material into table values
/// and resource/pipeline compatibility.
pub(crate) fn analytic_material_slot_candidate(
    base_material: &StandardMaterial,
    base_color: Color,
    alpha_mode: AlphaMode,
    lighting: Lighting,
    sidedness: Sidedness,
) -> MaterialSlotCandidate {
    let mut material = base_material.clone();
    material.base_color = base_color;
    material.alpha_mode = alpha_mode;
    material.unlit = matches!(lighting, Lighting::Unlit);
    render::apply_sidedness(&mut material, sidedness);
    MaterialSlotCandidate::from(&material)
}

/// Converts an absolute run record into the value uploaded for one batch.
///
/// `ClipDepthNudge` is uploaded relative to the batch's first
/// `DrawOrderIndex`; `OitDepthOffset` stays absolute to the panel because OIT
/// compares fragments across records.
pub(crate) fn path_render_record_relative_to_first_draw_order_index(
    mut record: PathRenderRecord,
    first_draw_order_index: DrawOrderIndex,
) -> PathRenderRecord {
    record.clip_depth_nudge -= first_draw_order_index.clip_depth_nudge().get();
    record
}

/// Dirty state for material-table row id changes in `PathRenderRecord`.
#[derive(Debug, Default)]
pub(crate) struct MaterialDirty {
    /// Whether the path render-record buffer needs a material-id upload.
    render_records: Dirty,
}

impl MaterialDirty {
    /// Marks `PathRenderRecord::material` data dirty.
    pub(crate) const fn mark(&mut self) { self.render_records.mark(); }

    /// Clears material-only render-record dirtiness after upload.
    pub(crate) const fn clear(&mut self) { self.render_records.clear(); }

    /// Whether material slot ids need a render-record upload.
    #[must_use]
    pub(crate) const fn is_set(&self) -> bool { self.render_records.is_set() }
}

/// Dirty state for placement, AA, render mode, and depth values.
#[derive(Debug, Default)]
pub(crate) struct PlacementDirty {
    /// Whether non-material `PathRenderRecord` data needs upload.
    render_records: Dirty,
    /// Whether batch bounds must be recomputed from placement changes.
    bounds:         Dirty,
}

impl PlacementDirty {
    /// Marks placement-sensitive render-record and bounds data dirty.
    pub(crate) const fn mark(&mut self) {
        self.render_records.mark();
        self.bounds.mark();
    }

    /// Clears placement render-record dirtiness after upload.
    pub(crate) const fn clear_render_records(&mut self) { self.render_records.clear(); }

    /// Clears placement bounds dirtiness after bounds recomputation.
    pub(crate) const fn clear_bounds(&mut self) { self.bounds.clear(); }

    /// Whether placement data needs a render-record upload.
    #[must_use]
    pub(crate) const fn render_records_are_dirty(&self) -> bool { self.render_records.is_set() }

    /// Whether placement data requires a bounds recomputation.
    #[must_use]
    pub(crate) const fn bounds_are_dirty(&self) -> bool { self.bounds.is_set() }
}

/// Dirty state for path quad records and packed-path atlas changes.
#[derive(Debug, Default)]
pub(crate) struct GeometryDirty {
    /// Whether the path quad buffer needs upload.
    path_quads: Dirty,
    /// Whether batch bounds must be recomputed from quad geometry changes.
    bounds:     Dirty,
}

impl GeometryDirty {
    /// Marks `PathQuadRecord` and bounds data dirty.
    pub(crate) const fn mark(&mut self) {
        self.path_quads.mark();
        self.bounds.mark();
    }

    /// Clears path-quad dirtiness after upload.
    pub(crate) const fn clear_path_quads(&mut self) { self.path_quads.clear(); }

    /// Clears geometry bounds dirtiness after bounds recomputation.
    pub(crate) const fn clear_bounds(&mut self) { self.bounds.clear(); }

    /// Whether path quad records need upload.
    #[must_use]
    pub(crate) const fn path_quads_are_dirty(&self) -> bool { self.path_quads.is_set() }

    /// Whether geometry data requires a bounds recomputation.
    #[must_use]
    pub(crate) const fn bounds_are_dirty(&self) -> bool { self.bounds.is_set() }
}

/// GPU-side handles for one batch, created by the routing system on the
/// batch's first frame.
///
/// Both record buffers are **padded to capacity on every upload** so their
/// byte length never changes between growths: bevy re-creates the wgpu buffer
/// when the length changes (`bevy_render/src/storage.rs` `prepare_asset`),
/// and the material's bind group would keep pointing at the old buffer —
/// whether a same-frame material re-prepare sees the new buffer is a prepare
/// -order race. Constant-length uploads always write the existing buffer in
/// place, which existing bind groups observe. A capacity growth creates new
/// buffer assets and rewrites the material's handles, which re-prepares
/// reliably (a missing render asset retries next frame).
#[derive(Debug)]
pub(crate) struct PathBatchResources {
    /// `PathQuadRecord` storage buffer (binding 104), `capacity` records.
    pub instances:    Handle<ShaderBuffer>,
    /// `PathRenderRecord` storage buffer (binding 105), `run_capacity` records.
    pub run_table:    Handle<ShaderBuffer>,
    /// Inert capacity-sized mesh; re-created and swapped on capacity growth.
    pub mesh:         Handle<Mesh>,
    /// The batch's material; its buffer handles are rewritten on growth.
    pub material:     Handle<PathExtendedMaterial>,
    /// Path-instance capacity of `mesh` and `instances`.
    pub capacity:     u32,
    /// Run-record capacity of `run_table`.
    pub run_capacity: u32,
}

/// One member run's contribution: the source data `rebuild` derives the
/// concatenated record vectors from, plus its derived range.
#[derive(Debug)]
struct BatchRun {
    key:              RunStorageKey,
    draw_order_index: DrawOrderIndex,
    /// Path records in run order; `render_index` is stamped by `rebuild`.
    path_records:     Vec<PathQuadRecord>,
    record:           PathRenderRecord,
    /// This run's slots in the concatenated path records — derived state,
    /// written only by `rebuild`.
    range:            Range<u32>,
}

/// One render entity + one material + one mesh per [`PathBatchKey`]: the CPU
/// record vectors the GPU tables upload from, the member runs they derive
/// from, and the split dirty flags the commit system reads.
#[derive(Debug, Default)]
pub(crate) struct TextRunBatch {
    /// The batch render entity; `None` until the routing system spawns it.
    pub entity:             Option<Entity>,
    /// GPU handles; `None` until the routing system creates them.
    pub gpu:                Option<PathBatchResources>,
    /// Material-slot row ids changed in `PathRenderRecord::material`.
    pub material_dirty:     MaterialDirty,
    /// Placement, render mode, AA, or depth changed in path render records.
    pub placement_dirty:    PlacementDirty,
    /// Path quad membership or geometry changed.
    pub geometry_dirty:     GeometryDirty,
    /// Lowest `DrawOrderIndex` in this batch.
    ///
    /// `PathRenderRecord::clip_depth_nudge` is uploaded relative to this value;
    /// `PathRenderRecord::oit_depth_offset` stays panel-absolute.
    first_draw_order_index: DrawOrderIndex,
    runs:                   Vec<BatchRun>,
    path_records:           Vec<PathQuadRecord>,
    run_records:            Vec<PathRenderRecord>,
}

impl TextRunBatch {
    /// Concatenated path records, `render_index` stamped — the instance-buffer
    /// upload payload.
    #[must_use]
    pub(in crate::render) fn path_records(&self) -> &[PathQuadRecord] { &self.path_records }

    /// One record per member run — the run-table upload payload.
    #[must_use]
    pub(in crate::render) fn run_records(&self) -> &[PathRenderRecord] { &self.run_records }

    /// Number of member runs.
    #[must_use]
    pub(in crate::render) const fn run_count(&self) -> usize { self.runs.len() }

    /// Number of path records across all member runs.
    #[must_use]
    pub(in crate::render) fn path_record_count(&self) -> u32 { self.path_records.len().to_u32() }

    /// Whether the last member run has left.
    #[must_use]
    const fn is_empty(&self) -> bool { self.runs.is_empty() }

    /// World-space bounds over every path rect × its run transform, for the
    /// Aabb-union system. `None` when the batch holds no records.
    #[must_use]
    pub(in crate::render) fn world_bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut min = Vec3::MAX;
        let mut max = Vec3::MIN;
        let mut any = false;
        for record in &self.path_records {
            let run = &self.run_records[record.render_index.to_usize()];
            for (corner_x, corner_y) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                let local = record.rect_min + Vec2::new(corner_x, corner_y) * record.rect_size;
                let world = run.transform * Vec4::new(local.x, local.y, 0.0, 1.0);
                min = min.min(world.xyz());
                max = max.max(world.xyz());
                any = true;
            }
        }
        any.then_some((min, max))
    }

    fn position_of(&self, key: RunStorageKey) -> Option<usize> {
        self.runs.iter().position(|run| run.key == key)
    }

    fn refresh_first_draw_order_index(&mut self) {
        self.first_draw_order_index = self
            .runs
            .iter()
            .map(|run| run.draw_order_index)
            .min()
            .unwrap_or_default();
    }

    /// Recomputes the concatenated record vectors, run indices, and ranges
    /// from the live run set. The sole writer of `range`, so ranges cannot go
    /// stale relative to the vectors they index. Every record field is
    /// stamped from the run's source data — never defaulted.
    fn rebuild(&mut self) {
        self.refresh_first_draw_order_index();
        self.path_records.clear();
        self.run_records.clear();
        let first_draw_order_index = self.first_draw_order_index;
        for (index, run) in self.runs.iter_mut().enumerate() {
            let start = self.path_records.len().to_u32();
            let render_index = index.to_u32();
            self.path_records
                .extend(run.path_records.iter().map(|record| PathQuadRecord {
                    render_index,
                    ..*record
                }));
            run.range = start..self.path_records.len().to_u32();
            self.run_records
                .push(path_render_record_relative_to_first_draw_order_index(
                    run.record,
                    first_draw_order_index,
                ));
        }
        self.geometry_dirty.mark();
        self.material_dirty.mark();
        self.placement_dirty.mark();
    }

    fn push_run(
        &mut self,
        key: RunStorageKey,
        draw_order_index: DrawOrderIndex,
        path_records: Vec<PathQuadRecord>,
        record: PathRenderRecord,
    ) {
        self.runs.push(BatchRun {
            key,
            draw_order_index,
            path_records,
            record,
            range: 0..0,
        });
        self.rebuild();
    }

    fn remove_run(&mut self, key: RunStorageKey) {
        if let Some(position) = self.position_of(key) {
            self.runs.remove(position);
            self.rebuild();
        }
    }

    /// Replaces a member run's records. A same-count edit (the steady-state
    /// stress case) writes the run's range in place; a count change takes the
    /// rebuild path.
    fn update_run(
        &mut self,
        key: RunStorageKey,
        draw_order_index: DrawOrderIndex,
        path_records: Vec<PathQuadRecord>,
        record: PathRenderRecord,
    ) {
        let Some(position) = self.position_of(key) else {
            return;
        };
        if self.runs[position].draw_order_index != draw_order_index {
            self.runs[position].draw_order_index = draw_order_index;
            self.runs[position].path_records = path_records;
            self.runs[position].record = record;
            self.rebuild();
            return;
        }
        if self.runs[position].path_records.len() == path_records.len() {
            let render_index = position.to_u32();
            let range = self.runs[position].range.clone();
            let mut path_records_changed = false;
            for (slot, source) in range.zip(path_records.iter()) {
                let index = slot.to_usize();
                let stamped = PathQuadRecord {
                    render_index,
                    ..*source
                };
                if self.path_records[index] != stamped {
                    self.path_records[index] = stamped;
                    path_records_changed = true;
                }
            }
            let batch_record = path_render_record_relative_to_first_draw_order_index(
                record,
                self.first_draw_order_index,
            );
            if self.run_records[position] != batch_record {
                self.run_records[position] = batch_record;
                self.material_dirty.mark();
                self.placement_dirty.mark();
            }
            self.runs[position].path_records = path_records;
            self.runs[position].record = record;
            if path_records_changed {
                self.geometry_dirty.mark();
            }
        } else {
            self.runs[position].path_records = path_records;
            self.runs[position].record = record;
            self.rebuild();
        }
    }

    /// Writes one member run's transform into its `PathRenderRecord` slot, dirtying
    /// the run table only when the matrix actually changed.
    fn update_run_transform(&mut self, key: RunStorageKey, transform: Mat4) {
        let Some(position) = self.position_of(key) else {
            return;
        };
        if self.runs[position].record.transform == transform {
            return;
        }
        self.runs[position].record.transform = transform;
        self.run_records[position].transform = transform;
        self.placement_dirty.mark();
    }

    /// Writes one member run's frame-local material slot into its
    /// `PathRenderRecord` without dirtying quads, atlas data, or bounds.
    fn update_run_material(&mut self, key: RunStorageKey, material: GpuMaterialSlotId) {
        let Some(position) = self.position_of(key) else {
            return;
        };
        if self.runs[position].record.material == material {
            return;
        }
        self.runs[position].record.material = material;
        self.run_records[position].material = material;
        self.material_dirty.mark();
    }

    /// Rewrites one member run's record without touching its glyph quads. Dirties
    /// the material buffer when the table row changed and the placement buffer
    /// when any other record field changed.
    fn update_run_record(&mut self, key: RunStorageKey, record: PathRenderRecord) {
        let Some(position) = self.position_of(key) else {
            return;
        };
        let batch_record = path_render_record_relative_to_first_draw_order_index(
            record,
            self.first_draw_order_index,
        );
        let current = self.run_records[position];
        if current == batch_record {
            return;
        }
        if current.material != batch_record.material {
            self.material_dirty.mark();
        }
        // Whole-record compares avoid per-field float equality: equalize the
        // material slot, and any remaining difference is a placement field.
        let mut placement_probe = batch_record;
        placement_probe.material = current.material;
        if placement_probe != current {
            self.placement_dirty.mark();
        }
        self.runs[position].record = record;
        self.run_records[position] = batch_record;
    }

    /// Whether either material or placement fields require a render-record upload.
    #[must_use]
    pub(in crate::render) const fn render_records_are_dirty(&self) -> bool {
        self.material_dirty.is_set() || self.placement_dirty.render_records_are_dirty()
    }

    /// Whether path quads require upload.
    #[must_use]
    pub(in crate::render) const fn path_quads_are_dirty(&self) -> bool {
        self.geometry_dirty.path_quads_are_dirty()
    }

    /// Whether the batch Aabb must be recomputed.
    #[must_use]
    pub(in crate::render) const fn bounds_are_dirty(&self) -> bool {
        self.geometry_dirty.bounds_are_dirty() || self.placement_dirty.bounds_are_dirty()
    }

    /// Clears render-record dirty state after upload.
    pub(in crate::render) const fn clear_render_record_dirty(&mut self) {
        self.material_dirty.clear();
        self.placement_dirty.clear_render_records();
    }

    /// Clears path-quad dirty state after upload.
    pub(in crate::render) const fn clear_path_quad_dirty(&mut self) {
        self.geometry_dirty.clear_path_quads();
    }

    /// Clears bounds dirty state after recomputation.
    pub(in crate::render) const fn clear_bounds_dirty(&mut self) {
        self.geometry_dirty.clear_bounds();
        self.placement_dirty.clear_bounds();
    }
}

impl BatchEntry for TextRunBatch {
    fn is_empty(&self) -> bool { Self::is_empty(self) }

    fn entity(&self) -> Option<Entity> { self.entity }
}

impl Batch for TextRunBatch {
    type MemberKey = RunStorageKey;
    type Payload = TextRunPayload;

    fn insert(&mut self, member: Self::MemberKey, payload: Self::Payload) {
        self.push_run(
            member,
            payload.draw_order_index,
            payload.path_records,
            payload.record,
        );
    }

    fn update(&mut self, member: Self::MemberKey, payload: Self::Payload) {
        self.update_run(
            member,
            payload.draw_order_index,
            payload.path_records,
            payload.record,
        );
    }

    fn remove(&mut self, member: Self::MemberKey) { self.remove_run(member); }
}

/// Routed text-run payload owned by `TextRunBatch`.
#[derive(Debug)]
pub(in crate::render) struct TextRunPayload {
    draw_order_index: DrawOrderIndex,
    path_records:     Vec<PathQuadRecord>,
    record:           PathRenderRecord,
}

/// Routes each text run to the analytic-path batch keyed by `PathBatchKey`.
#[derive(Debug, Default)]
pub(crate) struct TextRunBatchStore(BatchStore<PathBatchKey, TextRunBatch>);

impl TextRunBatchStore {
    /// Whether a run is currently routed to any batch.
    #[must_use]
    pub(crate) fn is_routed(&self, run: RunStorageKey) -> bool { self.0.contains(run) }

    /// Current batch key for a routed run.
    #[must_use]
    pub(crate) fn key_for_run(&self, run: RunStorageKey) -> Option<&PathBatchKey> {
        self.0.key_for(run)
    }

    /// Inserts a run into its key's batch, moves it when its key changed, or
    /// updates it in place — the single membership mutation point, together
    /// with [`Self::remove_run`].
    pub(crate) fn upsert_run(
        &mut self,
        key: PathBatchKey,
        run: RunStorageKey,
        draw_order_index: DrawOrderIndex,
        path_records: Vec<PathQuadRecord>,
        record: PathRenderRecord,
    ) {
        self.0.upsert(
            key,
            run,
            TextRunPayload {
                draw_order_index,
                path_records,
                record,
            },
        );
    }

    /// Removes a run from its batch. The emptied batch keeps its store entry
    /// until the routing system reconciles it via [`Self::take_empty_batches`].
    pub(crate) fn remove_run(&mut self, run: RunStorageKey) { self.0.remove(run); }

    /// Writes a routed run's world transform into its `PathRenderRecord` slot. A
    /// no-op for unrouted runs (e.g. a fully clipped label).
    pub(crate) fn update_run_transform(&mut self, run: RunStorageKey, transform: Mat4) {
        if let Some(batch) = self.0.member_batch_mut(run) {
            batch.update_run_transform(run, transform);
        }
    }

    /// Writes a routed run's frame-local material-table row id. A no-op for
    /// unrouted runs and for unchanged row ids.
    pub(crate) fn update_run_material(&mut self, run: RunStorageKey, material: MaterialSlotId) {
        if let Some(batch) = self.0.member_batch_mut(run) {
            batch.update_run_material(run, GpuMaterialSlotId::from(material));
        }
    }

    /// Rewrites a routed run's full render record without rebuilding its glyph
    /// quads. A no-op for unrouted runs.
    pub(crate) fn update_run_record(&mut self, run: RunStorageKey, record: PathRenderRecord) {
        if let Some(batch) = self.0.member_batch_mut(run) {
            batch.update_run_record(run, record);
        }
    }

    /// All batches.
    pub(crate) fn batches(&self) -> impl Iterator<Item = (&PathBatchKey, &TextRunBatch)> {
        self.0.batches()
    }

    /// All batches, mutable.
    pub(crate) fn batches_mut(
        &mut self,
    ) -> impl Iterator<Item = (&PathBatchKey, &mut TextRunBatch)> {
        self.0.batches_mut()
    }

    /// One batch by key.
    #[must_use]
    pub(crate) fn get(&self, key: &PathBatchKey) -> Option<&TextRunBatch> { self.0.get(key) }

    /// One batch by key, mutable.
    pub(crate) fn get_mut(&mut self, key: &PathBatchKey) -> Option<&mut TextRunBatch> {
        self.0.get_mut(key)
    }

    /// Drops batches whose last run left, returning their entities for the
    /// routing system to despawn (the batch analogue of the empty-run path).
    pub(crate) fn take_empty_batches(&mut self) -> Vec<Entity> { self.0.take_empty_batches() }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should fail loudly when fixture batches are missing"
)]
mod tests {
    use bevy::camera::visibility::RenderLayers;
    use bevy::color::Color;
    use bevy::prelude::AlphaMode;

    use super::*;
    use crate::render::material_table::MaterialSlotId;

    fn path_record(rect_min: Vec2, packed_path_index: u32) -> PathQuadRecord {
        PathQuadRecord {
            rect_min,
            rect_size: Vec2::ONE,
            uv_min: Vec2::ZERO,
            uv_size: Vec2::ONE,
            box_uv_min: Vec2::ZERO,
            box_uv_size: Vec2::ONE,
            packed_path_index,
            render_index: 0,
            box_uv_flip_x: 0,
        }
    }

    fn record(transform: Mat4) -> PathRenderRecord {
        PathRenderRecord {
            transform,
            material: GpuMaterialSlotId::from(
                MaterialSlotId::try_from(0).expect("slot 0 is valid"),
            ),
            render_mode: 1,
            clip_depth_nudge: 0.0,
            oit_depth_offset: 0.0,
            aa_flags: 3,
            text_coverage_bias: 0.0,
            text_em_height: 0.0,
        }
    }

    fn key(alpha: AlphaMode) -> PathBatchKey {
        let material = StandardMaterial {
            alpha_mode: alpha,
            ..Default::default()
        };
        PathBatchKey {
            z_index:                0.into(),
            z_index_rank:           DrawZIndexRank::default(),
            batch_family:           DrawBatchFamily::Text,
            shadow:                 VisualShadow::Cast,
            layers:                 BatchRenderLayers(RenderLayers::layer(0)),
            pipeline_compatibility: PipelineCompatibility::from(&material),
            resource_compatibility: ResourceCompatibility::from(&material),
        }
    }

    fn run_key(bits: u64) -> RunStorageKey { RunStorageKey::from(Entity::from_bits(bits)) }

    fn upsert_run(
        store: &mut TextRunBatchStore,
        batch_key: PathBatchKey,
        run: RunStorageKey,
        path_records: Vec<PathQuadRecord>,
        record: PathRenderRecord,
    ) {
        store.upsert_run(
            batch_key,
            run,
            DrawOrderIndex::default(),
            path_records,
            record,
        );
    }

    #[test]
    fn two_runs_one_key_share_a_batch_with_contiguous_ranges() {
        let mut store = TextRunBatchStore::default();
        let batch_key = key(AlphaMode::Blend);
        let first = run_key(1);
        let second = run_key(2);

        upsert_run(
            &mut store,
            batch_key.clone(),
            first,
            vec![path_record(Vec2::ZERO, 0), path_record(Vec2::X, 1)],
            record(Mat4::IDENTITY),
        );
        upsert_run(
            &mut store,
            batch_key.clone(),
            second,
            vec![path_record(Vec2::Y, 2)],
            record(Mat4::IDENTITY),
        );

        assert_eq!(store.batches().count(), 1);
        let batch = store.get(&batch_key).expect("batch should exist");
        assert_eq!(batch.run_count(), 2);
        assert_eq!(batch.path_record_count(), 3);
        // Records are concatenated in insertion order with run indices stamped.
        let stamped: Vec<u32> = batch
            .path_records()
            .iter()
            .map(|record| record.render_index)
            .collect();
        assert_eq!(stamped, vec![0, 0, 1]);
        assert!(batch.path_quads_are_dirty());
        assert!(batch.render_records_are_dirty());
        assert!(batch.bounds_are_dirty());
    }

    #[test]
    fn runs_differing_only_by_render_layers_route_to_separate_batches() {
        let mut store = TextRunBatchStore::default();
        let layer0 = key(AlphaMode::Blend);
        let layer1 = PathBatchKey {
            layers: BatchRenderLayers(RenderLayers::layer(1)),
            ..key(AlphaMode::Blend)
        };
        // Identical compatibility/order; the render layer is the only difference.
        assert_ne!(layer0, layer1);

        upsert_run(
            &mut store,
            layer0.clone(),
            run_key(1),
            vec![path_record(Vec2::ZERO, 0)],
            record(Mat4::IDENTITY),
        );
        upsert_run(
            &mut store,
            layer1.clone(),
            run_key(2),
            vec![path_record(Vec2::ZERO, 0)],
            record(Mat4::IDENTITY),
        );

        assert_eq!(store.batches().count(), 2);
        assert_eq!(store.get(&layer0).expect("layer 0 batch").run_count(), 1);
        assert_eq!(store.get(&layer1).expect("layer 1 batch").run_count(), 1);
    }

    #[test]
    fn removing_a_run_rebuilds_the_survivors_ranges() {
        let mut store = TextRunBatchStore::default();
        let batch_key = key(AlphaMode::Blend);
        let first = run_key(1);
        let second = run_key(2);
        upsert_run(
            &mut store,
            batch_key.clone(),
            first,
            vec![path_record(Vec2::ZERO, 0), path_record(Vec2::X, 1)],
            record(Mat4::IDENTITY),
        );
        upsert_run(
            &mut store,
            batch_key.clone(),
            second,
            vec![path_record(Vec2::Y, 2)],
            record(Mat4::IDENTITY),
        );

        store.remove_run(first);

        assert!(!store.is_routed(first));
        let batch = store.get(&batch_key).expect("batch should survive");
        assert_eq!(batch.run_count(), 1);
        assert_eq!(batch.path_record_count(), 1);
        // The surviving run shifted to index 0 and its records re-stamped.
        assert_eq!(batch.path_records()[0].render_index, 0);
        assert_eq!(batch.path_records()[0].packed_path_index, 2);
    }

    #[test]
    fn same_count_edit_writes_in_place_without_touching_the_run_table() {
        let mut store = TextRunBatchStore::default();
        let batch_key = key(AlphaMode::Blend);
        let run = run_key(1);
        let stamped = record(Mat4::IDENTITY);
        upsert_run(
            &mut store,
            batch_key.clone(),
            run,
            vec![path_record(Vec2::ZERO, 0), path_record(Vec2::X, 1)],
            stamped,
        );
        {
            let batch = store.get_mut(&batch_key).expect("batch should exist");
            batch.clear_path_quad_dirty();
            batch.clear_render_record_dirty();
        }

        // Same record count, different atlas indices — the stress-test edit
        // pattern ("07 412" → "07 413").
        upsert_run(
            &mut store,
            batch_key.clone(),
            run,
            vec![path_record(Vec2::ZERO, 5), path_record(Vec2::X, 6)],
            stamped,
        );

        let batch = store.get(&batch_key).expect("batch should exist");
        assert!(batch.path_quads_are_dirty(), "path records changed");
        assert!(
            batch.geometry_dirty.bounds_are_dirty(),
            "path record changes must recompute bounds"
        );
        assert!(
            !batch.render_records_are_dirty(),
            "an unchanged run record must not dirty the run table"
        );
        let atlas: Vec<u32> = batch
            .path_records()
            .iter()
            .map(|record| record.packed_path_index)
            .collect();
        assert_eq!(atlas, vec![5, 6]);
    }

    #[test]
    fn identical_same_count_quads_do_not_dirty_geometry() {
        let mut store = TextRunBatchStore::default();
        let batch_key = key(AlphaMode::Blend);
        let run = run_key(1);
        let path_records = vec![path_record(Vec2::ZERO, 0), path_record(Vec2::X, 1)];
        upsert_run(
            &mut store,
            batch_key.clone(),
            run,
            path_records.clone(),
            record(Mat4::IDENTITY),
        );
        {
            let batch = store.get_mut(&batch_key).expect("batch should exist");
            batch.clear_path_quad_dirty();
            batch.clear_render_record_dirty();
            batch.clear_bounds_dirty();
        }

        let mut updated = record(Mat4::from_translation(Vec3::X));
        updated.material =
            GpuMaterialSlotId::from(MaterialSlotId::try_from(7).expect("slot 7 is valid"));
        upsert_run(&mut store, batch_key.clone(), run, path_records, updated);

        let batch = store.get(&batch_key).expect("batch should exist");
        assert!(batch.material_dirty.is_set());
        assert!(batch.placement_dirty.render_records_are_dirty());
        assert!(
            batch.render_records_are_dirty(),
            "material and placement edits update the run table"
        );
        assert!(
            !batch.path_quads_are_dirty(),
            "identical path records must not upload the quad table"
        );
        assert!(
            !batch.geometry_dirty.bounds_are_dirty(),
            "identical path records must not mark geometry bounds dirty"
        );
        assert!(
            batch.placement_dirty.bounds_are_dirty(),
            "placement changes still recompute bounds"
        );
        assert_eq!(batch.run_records()[0], updated);
    }

    #[test]
    fn count_change_takes_the_rebuild_path() {
        let mut store = TextRunBatchStore::default();
        let batch_key = key(AlphaMode::Blend);
        let run = run_key(1);
        upsert_run(
            &mut store,
            batch_key.clone(),
            run,
            vec![path_record(Vec2::ZERO, 0)],
            record(Mat4::IDENTITY),
        );

        upsert_run(
            &mut store,
            batch_key.clone(),
            run,
            vec![path_record(Vec2::ZERO, 0), path_record(Vec2::X, 1)],
            record(Mat4::IDENTITY),
        );

        let batch = store.get(&batch_key).expect("batch should exist");
        assert_eq!(batch.path_record_count(), 2);
        assert!(
            batch.render_records_are_dirty(),
            "a rebuild re-uploads both buffers"
        );
    }

    #[test]
    fn key_change_moves_the_run_between_batches() {
        let mut store = TextRunBatchStore::default();
        let blend = key(AlphaMode::Blend);
        let add = key(AlphaMode::Add);
        let run = run_key(1);
        upsert_run(
            &mut store,
            blend.clone(),
            run,
            vec![path_record(Vec2::ZERO, 0)],
            record(Mat4::IDENTITY),
        );

        upsert_run(
            &mut store,
            add.clone(),
            run,
            vec![path_record(Vec2::ZERO, 0)],
            record(Mat4::IDENTITY),
        );

        let source = store.get(&blend).expect("source batch entry persists");
        assert!(source.is_empty(), "the run left its old batch");
        let destination = store.get(&add).expect("destination batch exists");
        assert_eq!(destination.run_count(), 1);
        let emptied = store.take_empty_batches();
        assert!(emptied.is_empty(), "no entity was ever spawned");
        assert!(store.get(&blend).is_none(), "the emptied entry is dropped");
    }

    #[test]
    fn move_then_remove_in_one_pass_leaves_both_maps_consistent() {
        let mut store = TextRunBatchStore::default();
        let blend = key(AlphaMode::Blend);
        let add = key(AlphaMode::Add);
        let run = run_key(1);
        upsert_run(
            &mut store,
            blend,
            run,
            vec![path_record(Vec2::ZERO, 0)],
            record(Mat4::IDENTITY),
        );

        // One routing pass: the run re-keys (a live cascade change) and is
        // removed (its label despawned) before any reconcile runs.
        upsert_run(
            &mut store,
            add,
            run,
            vec![path_record(Vec2::ZERO, 0)],
            record(Mat4::IDENTITY),
        );
        store.remove_run(run);

        assert!(!store.is_routed(run));
        let total_runs: usize = store.batches().map(|(_, batch)| batch.run_count()).sum();
        assert_eq!(total_runs, 0, "no batch retains the removed run");
        store.take_empty_batches();
        assert_eq!(
            store.batches().count(),
            0,
            "both the source and destination entries empty out and drop"
        );
    }

    #[test]
    fn transform_update_dirties_the_run_table_only_on_change() {
        let mut store = TextRunBatchStore::default();
        let batch_key = key(AlphaMode::Blend);
        let run = run_key(1);
        upsert_run(
            &mut store,
            batch_key.clone(),
            run,
            vec![path_record(Vec2::ZERO, 0)],
            record(Mat4::IDENTITY),
        );
        {
            let batch = store.get_mut(&batch_key).expect("batch should exist");
            batch.clear_render_record_dirty();
            batch.clear_bounds_dirty();
        }

        store.update_run_transform(run, Mat4::IDENTITY);
        let batch = store.get(&batch_key).expect("batch should exist");
        assert!(
            !batch.render_records_are_dirty(),
            "an identical matrix must not dirty the run table"
        );

        let moved = Mat4::from_translation(Vec3::X);
        store.update_run_transform(run, moved);
        let batch = store.get(&batch_key).expect("batch should exist");
        assert!(batch.render_records_are_dirty());
        assert!(batch.bounds_are_dirty());
        assert_eq!(batch.run_records()[0].transform, moved);
    }

    #[test]
    fn scalar_material_values_do_not_enter_path_batch_key() {
        let first = StandardMaterial {
            base_color: Color::srgb(0.5, 0.2, 0.2),
            metallic: 0.25,
            perceptual_roughness: 0.3,
            ..Default::default()
        };
        let second = StandardMaterial {
            base_color: Color::srgb(0.1, 0.7, 0.4),
            metallic: 0.9,
            perceptual_roughness: 0.8,
            ..Default::default()
        };

        assert_eq!(
            PipelineCompatibility::from(&first),
            PipelineCompatibility::from(&second)
        );
        assert_eq!(
            ResourceCompatibility::from(&first),
            ResourceCompatibility::from(&second)
        );
    }

    #[test]
    fn world_bounds_unions_rects_across_run_transforms() {
        let mut store = TextRunBatchStore::default();
        let batch_key = key(AlphaMode::Blend);
        upsert_run(
            &mut store,
            batch_key.clone(),
            run_key(1),
            vec![path_record(Vec2::ZERO, 0)],
            record(Mat4::IDENTITY),
        );
        upsert_run(
            &mut store,
            batch_key.clone(),
            run_key(2),
            vec![path_record(Vec2::ZERO, 0)],
            record(Mat4::from_translation(Vec3::new(4.0, 0.0, -2.0))),
        );

        let (min, max) = store
            .get(&batch_key)
            .expect("batch should exist")
            .world_bounds()
            .expect("two records produce bounds");

        // Unit rects at the origin and at (4, 0, -2).
        assert_eq!(min, Vec3::new(0.0, 0.0, -2.0));
        assert_eq!(max, Vec3::new(5.0, 1.0, 0.0));
    }

    #[test]
    fn material_slot_refresh_dirties_only_render_records() {
        let mut store = TextRunBatchStore::default();
        let batch_key = key(AlphaMode::Blend);
        let run = run_key(1);
        upsert_run(
            &mut store,
            batch_key.clone(),
            run,
            vec![path_record(Vec2::ZERO, 0)],
            record(Mat4::IDENTITY),
        );
        {
            let batch = store.get_mut(&batch_key).expect("batch should exist");
            batch.clear_path_quad_dirty();
            batch.clear_render_record_dirty();
            batch.clear_bounds_dirty();
        }

        store.update_run_material(run, MaterialSlotId::try_from(7).expect("slot 7 is valid"));

        let batch = store.get(&batch_key).expect("batch should exist");
        assert!(batch.material_dirty.is_set());
        assert!(batch.render_records_are_dirty());
        assert!(!batch.path_quads_are_dirty());
        assert!(!batch.bounds_are_dirty());
        assert_eq!(batch.run_records()[0].material.as_u32(), 7);
    }

    #[test]
    fn record_refresh_rewrites_the_run_table_without_touching_glyph_quads() {
        let mut store = TextRunBatchStore::default();
        let batch_key = key(AlphaMode::Blend);
        let run = run_key(1);
        upsert_run(
            &mut store,
            batch_key.clone(),
            run,
            vec![path_record(Vec2::ZERO, 0)],
            record(Mat4::IDENTITY),
        );
        let quads_before = store
            .get(&batch_key)
            .expect("batch should exist")
            .path_records()
            .to_vec();
        {
            let batch = store.get_mut(&batch_key).expect("batch should exist");
            batch.clear_path_quad_dirty();
            batch.clear_render_record_dirty();
            batch.clear_bounds_dirty();
        }

        // A render-only edit: new material row and render mode, identical quads.
        let mut refreshed = record(Mat4::IDENTITY);
        refreshed.material =
            GpuMaterialSlotId::from(MaterialSlotId::try_from(5).expect("slot 5 is valid"));
        refreshed.render_mode = 2;
        store.update_run_record(run, refreshed);

        let batch = store.get(&batch_key).expect("batch should exist");
        assert!(
            batch.material_dirty.is_set(),
            "a changed material row dirties the material buffer"
        );
        assert!(
            batch.placement_dirty.render_records_are_dirty(),
            "a changed render mode dirties the placement records"
        );
        assert!(
            !batch.path_quads_are_dirty(),
            "a render-only edit must not re-upload glyph quads"
        );
        assert_eq!(batch.path_records(), quads_before.as_slice());
        assert_eq!(batch.run_records()[0].material.as_u32(), 5);
        assert_eq!(batch.run_records()[0].render_mode, 2);
    }
}
