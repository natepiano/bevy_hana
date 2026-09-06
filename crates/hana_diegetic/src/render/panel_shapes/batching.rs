//! Batched analytic-path rendering for panel-owned line primitives.
//!
//! Every visible resolved line primitive becomes one analytic path instance and
//! one run record. Compatible records from any number of panels share one batch
//! render entity, one inert quad mesh, one `PathExtendedMaterial`, and one path atlas.

use std::collections::HashMap;

use bevy::asset::AssetId;
use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::system::SystemParam;
use bevy::light::NotShadowCaster;
use bevy::math::Vec2;
use bevy::math::Vec3;
use bevy::math::Vec3A;
use bevy::mesh::Indices;
use bevy::picking::Pickable;
use bevy::prelude::AlphaMode;
use bevy::prelude::AssetServer;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Handle;
use bevy::prelude::Mat4;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::RemovedComponents;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy::render::render_resource::PrimitiveTopology;
use bevy::render::storage::ShaderBuffer;
use hana_kana::ToF32;
use hana_kana::ToU32;
use hana_kana::ToUsize;

use super::path;
use super::path::PanelShapeMember;
use super::path::PanelShapePath;
use super::path::PanelShapePathContext;
use super::primitive::PanelShapeRenderKey;
use super::relationship::PanelShape;
use super::relationship::PanelShapeOf;
use super::relationship::PanelShapeSource;
use super::relationship::PanelShapes;
use crate::cascade::Resolved;
use crate::cascade::ShapeMaterial;
use crate::layout::BoundingBox;
use crate::layout::DrawBatchFamily;
use crate::layout::Lighting;
use crate::layout::PanelShapeSourceKey;
use crate::layout::RenderCommand;
use crate::layout::RenderCommandKind;
use crate::layout::ResolvedPanelShape;
use crate::layout::ResolvedPanelShapePrimitive;
use crate::layout::ShadowCasting;
use crate::layout::Sidedness;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::DiegeticPanel;
use crate::panel::DiegeticPerfStats;
use crate::panel::PanelLayout;
use crate::panel::PanelOwned;
use crate::render;
use crate::render::AntiAlias;
use crate::render::BatchRenderLayers;
use crate::render::Dirty;
use crate::render::GeometryDirty;
use crate::render::HairlineFade;
use crate::render::HairlineWidth;
use crate::render::MaterialDirty;
use crate::render::PathAtlas;
use crate::render::PathBatchKey;
use crate::render::PathExtendedMaterial;
use crate::render::PathMaterialBuffers;
use crate::render::PathOutline;
use crate::render::PathQuadRecord;
use crate::render::PathRenderRecord;
use crate::render::PlacementDirty;
use crate::render::RenderMode;
use crate::render::VisualShadow;
use crate::render::analytic_paths;
use crate::render::analytic_paths::PathAtlasHandles;
use crate::render::batch_key;
use crate::render::batch_store::Batch;
use crate::render::batch_store::BatchEntry;
use crate::render::batch_store::BatchStore;
use crate::render::draw_order::DrawCommandDepth;
use crate::render::draw_order::DrawOrder;
use crate::render::draw_order::DrawOrderIndex;
use crate::render::material_table;
use crate::render::material_table::FrameMaterialTableBuild;
use crate::render::material_table::FrameMaterialTableBuilder;
use crate::render::material_table::MaterialSlotAppend;
use crate::render::material_table::MaterialSlotCandidate;
use crate::render::material_table::MaterialSlotInput;
use crate::render::material_table::RetainedBatchBufferUploads;
use crate::render::material_table::SdfPaintMaterial;
use crate::widgets::VisualOverrideIndex;
use crate::widgets::VisualSlotOverride;

/// Target design-unit extent for one panel-line band (≈ 5.8mm at the
/// reference design scale). Bands shrink the per-fragment curve loop — a
/// merged ruler path carries hundreds of curves — but the banded distance
/// scan reaches only as far as the band overlap (half this extent), so bands
/// smaller than the on-screen scan width collapse the AA ramp to a hard step.
/// At this size a single tick or arrowhead still packs one exact band, and the
/// scan falls short only once a whole millimeter ruler drops under ~200 screen
/// pixels, where 1mm ticks are sub-pixel mush regardless.
const PANEL_LINE_BAND_TARGET_DESIGN_UNITS: f32 = 2048.0;

const PANEL_LINE_LINE_DEPTH_BIAS_STEP: f32 = 0.001;
const PANEL_LINE_PART_DEPTH_BIAS_STEP: f32 = 0.000_001;
const PANEL_LINE_LINE_OIT_DEPTH_STEP: f32 = 0.000_000_1;
const PANEL_LINE_PART_OIT_DEPTH_STEP: f32 = 0.000_000_001;

/// Marker on every panel-line batch render entity.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub(super) struct DiegeticPanelShapeBatch;

/// Append-time material input for one panel-line render record.
struct PanelShapeMaterialSlotInput<'a> {
    /// Source identity returned with the retained slot.
    key:           PanelShapeRenderKey,
    /// Resolved primitive/element/panel/default material before color override.
    base_material: &'a StandardMaterial,
    /// Resolved primitive color used as the row's effective `base_color`.
    fill_color:    Color,
    /// Current panel-line alpha pipeline mode.
    alpha_mode:    AlphaMode,
    /// Resolved panel-line lighting policy.
    lighting:      Lighting,
    /// Resolved panel-line sidedness policy.
    sidedness:     Sidedness,
}

impl MaterialSlotInput for PanelShapeMaterialSlotInput<'_> {
    type Key = PanelShapeRenderKey;

    fn key(&self) -> Self::Key { self.key }

    fn material_slot_candidate(&self) -> MaterialSlotCandidate {
        render::analytic_material_slot_candidate(
            self.base_material,
            self.fill_color,
            self.alpha_mode,
            self.lighting,
            self.sidedness,
        )
    }
}

/// GPU-side handles for one line path batch.
#[derive(Debug)]
struct ShapeBatchGpu {
    instances:    Handle<ShaderBuffer>,
    run_table:    Handle<ShaderBuffer>,
    mesh:         Handle<Mesh>,
    material:     Handle<PathExtendedMaterial>,
    capacity:     u32,
    run_capacity: u32,
}

/// One member primitive in a batch.
#[derive(Clone, Debug)]
struct ShapeBatchRecord {
    key:              PanelShapeRenderKey,
    draw_order_index: DrawOrderIndex,
    outline:          PathOutline,
    instance:         PathQuadRecord,
    run:              PathRenderRecord,
}

/// One render entity + material + mesh per [`PathBatchKey`].
#[derive(Debug, Default)]
struct ShapeBatch {
    entity:                 Option<Entity>,
    gpu:                    Option<ShapeBatchGpu>,
    material_dirty:         MaterialDirty,
    placement_dirty:        PlacementDirty,
    geometry_dirty:         GeometryDirty,
    /// Lowest `DrawOrderIndex` in this batch.
    ///
    /// `PathRenderRecord::clip_depth_nudge` is uploaded relative to this value;
    /// `PathRenderRecord::oit_depth_offset` stays panel-absolute.
    first_draw_order_index: DrawOrderIndex,
    records:                Vec<ShapeBatchRecord>,
}

impl ShapeBatch {
    const fn is_empty(&self) -> bool { self.records.is_empty() }

    fn record_count(&self) -> u32 { self.records.len().to_u32() }

    fn run_count(&self) -> u32 { self.records.len().to_u32() }

    fn instances(&self) -> Vec<PathQuadRecord> {
        self.records
            .iter()
            .enumerate()
            .map(|(index, record)| PathQuadRecord {
                render_index: index.to_u32(),
                ..record.instance
            })
            .collect()
    }

    fn run_records(&self) -> Vec<PathRenderRecord> {
        self.records
            .iter()
            .map(|record| {
                analytic_paths::path_render_record_relative_to_first_draw_order_index(
                    record.run,
                    self.first_draw_order_index,
                )
            })
            .collect()
    }

    fn refresh_first_draw_order_index(&mut self) {
        let previous = self.first_draw_order_index;
        self.first_draw_order_index = self
            .records
            .iter()
            .map(|record| record.draw_order_index)
            .min()
            .unwrap_or_default();
        if self.first_draw_order_index != previous {
            self.placement_dirty.mark();
        }
    }

    fn push_record(&mut self, record: ShapeBatchRecord) {
        self.records.push(record);
        self.refresh_first_draw_order_index();
        self.material_dirty.mark();
        self.placement_dirty.mark();
        self.geometry_dirty.mark();
    }

    fn remove_record(&mut self, key: PanelShapeRenderKey) {
        if let Some(index) = self.records.iter().position(|record| record.key == key) {
            self.records.remove(index);
            self.refresh_first_draw_order_index();
            self.material_dirty.mark();
            self.placement_dirty.mark();
            self.geometry_dirty.mark();
        }
    }

    fn position_of(&self, key: PanelShapeRenderKey) -> Option<usize> {
        self.records.iter().position(|record| record.key == key)
    }

    fn refresh_record(&mut self, incoming: ShapeBatchRecord) {
        let Some(index) = self.position_of(incoming.key) else {
            self.push_record(incoming);
            return;
        };
        let mut refresh_first_draw_order_index = false;
        {
            let record = &mut self.records[index];
            if record.outline != incoming.outline
                || !path_quad_geometry_eq(&record.instance, &incoming.instance)
            {
                record.draw_order_index = incoming.draw_order_index;
                record.outline = incoming.outline;
                let packed_path_index = record.instance.packed_path_index;
                record.instance = PathQuadRecord {
                    packed_path_index,
                    ..incoming.instance
                };
                record.run = incoming.run;
                self.material_dirty.mark();
                self.placement_dirty.mark();
                self.geometry_dirty.mark();
                refresh_first_draw_order_index = true;
            } else {
                if record.draw_order_index != incoming.draw_order_index {
                    record.draw_order_index = incoming.draw_order_index;
                    refresh_first_draw_order_index = true;
                }
                if path_render_record_placement_eq(&record.run, &incoming.run) {
                    if record.run.material != incoming.run.material {
                        record.run.material = incoming.run.material;
                        self.material_dirty.mark();
                    }
                } else {
                    record.run = incoming.run;
                    self.material_dirty.mark();
                    self.placement_dirty.mark();
                }
            }
        }
        if refresh_first_draw_order_index {
            self.refresh_first_draw_order_index();
        }
    }

    const fn render_records_are_dirty(&self) -> bool {
        self.material_dirty.is_set() || self.placement_dirty.render_records_are_dirty()
    }

    const fn path_quads_are_dirty(&self) -> bool { self.geometry_dirty.path_quads_are_dirty() }

    const fn bounds_are_dirty(&self) -> bool {
        self.geometry_dirty.bounds_are_dirty() || self.placement_dirty.bounds_are_dirty()
    }

    const fn clear_render_record_dirty(&mut self) {
        self.material_dirty.clear();
        self.placement_dirty.clear_render_records();
    }

    const fn clear_path_quad_dirty(&mut self) { self.geometry_dirty.clear_path_quads(); }

    const fn clear_bounds_dirty(&mut self) {
        self.geometry_dirty.clear_bounds();
        self.placement_dirty.clear_bounds();
    }

    fn world_bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut min = Vec3::MAX;
        let mut max = Vec3::MIN;
        let mut any = false;
        for record in &self.records {
            for (corner_x, corner_y) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                let local = record.instance.rect_min
                    + Vec2::new(corner_x, corner_y) * record.instance.rect_size;
                let world = record
                    .run
                    .transform
                    .transform_point3(Vec3::new(local.x, local.y, 0.0));
                min = min.min(world);
                max = max.max(world);
                any = true;
            }
        }
        any.then_some((min, max))
    }
}

impl BatchEntry for ShapeBatch {
    fn is_empty(&self) -> bool { Self::is_empty(self) }

    fn entity(&self) -> Option<Entity> { self.entity }
}

impl Batch for ShapeBatch {
    type MemberKey = PanelShapeRenderKey;
    type Payload = ShapeBatchRecord;

    fn insert(&mut self, member: Self::MemberKey, payload: Self::Payload) {
        debug_assert_eq!(member, payload.key);
        self.push_record(payload);
    }

    fn update(&mut self, member: Self::MemberKey, payload: Self::Payload) {
        debug_assert_eq!(member, payload.key);
        self.refresh_record(payload);
    }

    fn remove(&mut self, member: Self::MemberKey) { self.remove_record(member); }
}

fn path_quad_geometry_eq(left: &PathQuadRecord, right: &PathQuadRecord) -> bool {
    left.rect_min == right.rect_min
        && left.rect_size == right.rect_size
        && left.uv_min == right.uv_min
        && left.uv_size == right.uv_size
        && left.box_uv_min == right.box_uv_min
        && left.box_uv_size == right.box_uv_size
        && left.box_uv_flip_x == right.box_uv_flip_x
}

fn path_render_record_placement_eq(left: &PathRenderRecord, right: &PathRenderRecord) -> bool {
    left.transform == right.transform
        && left.render_mode == right.render_mode
        && left.clip_depth_nudge.to_bits() == right.clip_depth_nudge.to_bits()
        && left.oit_depth_offset.to_bits() == right.oit_depth_offset.to_bits()
        && left.aa_flags == right.aa_flags
}

/// Routes panel-line primitives into compatible cross-panel batches.
#[derive(Debug, Default, Resource)]
pub(super) struct ShapeBatchStore {
    store:         BatchStore<PathBatchKey, ShapeBatch>,
    /// Panel-scoped retain bookkeeping only. Batch keys live in `store`.
    panel_members: HashMap<Entity, Vec<PanelShapeRenderKey>>,
    atlas:         PathAtlas<PanelShapeRenderKey>,
    atlas_dirty:   Dirty,
}

impl ShapeBatchStore {
    fn upsert_panel(&mut self, panel: Entity, records: Vec<(PathBatchKey, ShapeBatchRecord)>) {
        if self.try_refresh_panel(panel, &records) {
            return;
        }
        let incoming_members: Vec<PanelShapeRenderKey> =
            records.iter().map(|(_, record)| record.key).collect();
        let stale_members: Vec<PanelShapeRenderKey> =
            self.panel_members
                .get(&panel)
                .map_or_else(Vec::new, |members| {
                    members
                        .iter()
                        .copied()
                        .filter(|member| !incoming_members.contains(member))
                        .collect()
                });
        let mut atlas_update = Dirty::No;
        if !stale_members.is_empty() {
            atlas_update.mark();
        }
        for member in stale_members {
            self.store.remove(member);
        }
        for (key, record) in records {
            let record_key = record.key;
            let current_key = self.store.key_for(record_key).cloned();
            if current_key.as_ref() != Some(&key) {
                atlas_update.mark();
            }
            let was_geometry_dirty = self
                .store
                .get(&key)
                .is_some_and(ShapeBatch::path_quads_are_dirty);
            self.store.upsert(key.clone(), record_key, record);
            if self
                .store
                .get(&key)
                .is_some_and(ShapeBatch::path_quads_are_dirty)
                && !was_geometry_dirty
            {
                atlas_update.mark();
            }
        }
        if incoming_members.is_empty() {
            self.panel_members.remove(&panel);
        } else {
            self.panel_members.insert(panel, incoming_members);
        }
        if atlas_update.is_set() {
            self.atlas_dirty.mark();
        }
    }

    fn try_refresh_panel(
        &mut self,
        panel: Entity,
        records: &[(PathBatchKey, ShapeBatchRecord)],
    ) -> bool {
        let Some(existing) = self.panel_members.get(&panel) else {
            return false;
        };
        if existing.len() != records.len() {
            return false;
        }
        if existing
            .iter()
            .zip(records)
            .any(|(old_record_key, (new_key, new_record))| {
                self.store.key_for(*old_record_key) != Some(new_key)
                    || *old_record_key != new_record.key
            })
        {
            return false;
        }
        for (key, record) in records.iter().cloned() {
            let Some(batch) = self.store.get_mut(&key) else {
                return false;
            };
            let was_geometry_dirty = batch.path_quads_are_dirty();
            batch.refresh_record(record);
            if batch.path_quads_are_dirty() && !was_geometry_dirty {
                self.atlas_dirty.mark();
            }
        }
        true
    }

    fn remove_panel(&mut self, panel: Entity) {
        let Some(records) = self.panel_members.remove(&panel) else {
            return;
        };
        self.atlas_dirty.mark();
        for record_key in records {
            self.store.remove(record_key);
        }
    }

    fn rebuild_path_atlas_if_dirty(&mut self) {
        if !self.atlas_dirty.is_set() {
            return;
        }
        let paths: Vec<(PanelShapeRenderKey, PathOutline)> = self
            .store
            .batches()
            .flat_map(|(_, batch)| {
                batch
                    .records
                    .iter()
                    .map(|record| (record.key, record.outline.clone()))
            })
            .collect();
        self.atlas
            .rebuild(paths, PANEL_LINE_BAND_TARGET_DESIGN_UNITS);
        for (_, batch) in self.store.batches_mut() {
            for record in &mut batch.records {
                if let Some(packed_path_index) = self.atlas.index(&record.key) {
                    record.instance.packed_path_index = packed_path_index;
                }
            }
            batch.geometry_dirty.mark();
        }
        self.atlas_dirty.clear();
    }

    fn commit_path_atlas(
        &mut self,
        storage_buffers: &mut Assets<ShaderBuffer>,
        materials: &mut Assets<PathExtendedMaterial>,
    ) -> Option<PathAtlasHandles> {
        let (atlas, uploaded) = self.atlas.upload(storage_buffers)?;
        if uploaded {
            for (_, batch) in self.store.batches() {
                let Some(gpu) = &batch.gpu else {
                    continue;
                };
                if let Some(mut material) = materials.get_mut(&gpu.material) {
                    render::set_path_material_atlas(
                        &mut material,
                        atlas.curves.clone(),
                        atlas.bands.clone(),
                        atlas.path_records.clone(),
                    );
                }
            }
        }
        Some(atlas)
    }

    fn take_empty_batches(&mut self) -> Vec<Entity> { self.store.take_empty_batches() }

    fn batches(&self) -> impl Iterator<Item = (&PathBatchKey, &ShapeBatch)> { self.store.batches() }

    fn batches_mut(&mut self) -> impl Iterator<Item = (&PathBatchKey, &mut ShapeBatch)> {
        self.store.batches_mut()
    }

    fn get(&self, key: &PathBatchKey) -> Option<&ShapeBatch> { self.store.get(key) }

    fn get_mut(&mut self, key: &PathBatchKey) -> Option<&mut ShapeBatch> { self.store.get_mut(key) }
}

struct PanelShapeReconcileContext<'a> {
    panel_entity:         Entity,
    panel:                &'a DiegeticPanel,
    /// Current `StandardMaterial` assets used to project source handles.
    standard_materials:   &'a Assets<StandardMaterial>,
    /// Seeded shape-material cascade default used as the final source handle.
    shape_default:        &'a ShapeMaterial,
    /// Panel's cascade-resolved panel-shape material handle.
    shape_material:       Handle<StandardMaterial>,
    asset_server:         &'a AssetServer,
    panel_transform:      Mat4,
    path_context:         PanelShapePathContext,
    /// Panel-level shadow casting resolved by the cascade.
    panel_shadow_casting: ShadowCasting,
    layers:               BatchRenderLayers,
    /// The panel entity's cascade-resolved lighting mode; every line on the
    /// panel renders with it, matching the panel's glyph runs.
    panel_lighting:       Lighting,
    /// The panel entity's cascade-resolved sidedness; every line on the panel
    /// renders with it, matching the panel's glyph runs.
    panel_sidedness:      Sidedness,
    /// The panel entity's cascade-resolved anti-alias mode; elements without
    /// their own override inherit it.
    panel_anti_alias:     AntiAlias,
    /// The panel entity's cascade-resolved hairline fade policy; elements
    /// without their own override inherit it.
    panel_hairline_fade:  HairlineFade,
}

#[derive(SystemParam)]
pub(super) struct PanelShapeMaterialAssets<'w> {
    standard_materials: Res<'w, Assets<StandardMaterial>>,
    asset_server:       Res<'w, AssetServer>,
    shape_default:      Res<'w, ShapeMaterial>,
    visual_overrides:   Res<'w, VisualOverrideIndex>,
}

struct ShapePrimitiveSource<'a> {
    element_index: usize,
    draw_depth:    DrawCommandDepth,
    line:          &'a ResolvedPanelShape,
    primitive:     &'a ResolvedPanelShapePrimitive,
}

struct BuiltPanelShapePrimitive {
    batch_key: PathBatchKey,
    record:    ShapeBatchRecord,
}

/// Same-silhouette grouping key: primitives that agree on everything that
/// must be uniform across one merged analytic path. Members of one group
/// render as a single multi-contour path, so abutting strokes (tick meets
/// spine) share one winding field and one anti-aliasing ramp.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PanelShapeMergeKey {
    element_index:     usize,
    clip:              Option<[u32; 4]>,
    owner_bounds:      [u32; 4],
    layering:          [u32; 2],
    material_identity: PanelShapeMaterialIdentity,
    shadow:            VisualShadow,
}

/// Opaque material-source facts that require separate `PathRenderRecord` rows.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PanelShapeMaterialIdentity {
    /// Resolved `StandardMaterial` asset id used before `FrameMaterialTable`
    /// row assignment.
    source_material: AssetId<StandardMaterial>,
    /// Authored base-color source identity when the color is not the default.
    base_color:      PanelShapeBaseColorIdentity,
}

/// Stable source for a panel-shape `base_color` override.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum PanelShapeBaseColorIdentity {
    /// The source uses the default white shape color.
    Default,
    /// The source authored a non-default color.
    Source(PanelShapeColorDiscriminator),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PanelShapeColorDiscriminator([u32; 4]);

impl From<Color> for PanelShapeColorDiscriminator {
    fn from(color: Color) -> Self { Self(color_bits(color)) }
}

impl PanelShapeMaterialIdentity {
    fn from_source(
        source: &ShapePrimitiveSource<'_>,
        source_material: &Handle<StandardMaterial>,
    ) -> Self {
        Self {
            source_material: source_material.id(),
            base_color:      PanelShapeBaseColorIdentity::from_color(source.primitive.color()),
        }
    }
}

impl PanelShapeBaseColorIdentity {
    fn from_color(color: Color) -> Self {
        if default_shape_color(color) {
            Self::Default
        } else {
            Self::Source(color.into())
        }
    }
}

impl PanelShapeMergeKey {
    fn from_source(
        source: &ShapePrimitiveSource<'_>,
        source_material: &Handle<StandardMaterial>,
        shadow: VisualShadow,
    ) -> Self {
        Self {
            element_index: source.element_index,
            clip: source.primitive.clip().map(bounding_box_bits),
            owner_bounds: bounding_box_bits(source.line.owner_bounds()),
            layering: [
                source.draw_depth.clip_depth_nudge().get().to_bits(),
                source.draw_depth.oit_depth_offset().get().to_bits(),
            ],
            material_identity: PanelShapeMaterialIdentity::from_source(source, source_material),
            shadow,
        }
    }
}

fn color_bits(color: Color) -> [u32; 4] {
    let linear = color.to_linear();
    [
        linear.red.to_bits(),
        linear.green.to_bits(),
        linear.blue.to_bits(),
        linear.alpha.to_bits(),
    ]
}

fn default_shape_color(color: Color) -> bool { color_bits(color) == color_bits(Color::WHITE) }

const fn bounding_box_bits(bounds: BoundingBox) -> [u32; 4] {
    [
        bounds.x.to_bits(),
        bounds.y.to_bits(),
        bounds.width.to_bits(),
        bounds.height.to_bits(),
    ]
}

/// Splits the primitive list into merge groups, preserving first-occurrence
/// order so group identities stay stable across reconciles.
fn group_line_primitives<'a>(
    sources: Vec<ShapePrimitiveSource<'a>>,
    context: &PanelShapeReconcileContext<'_>,
) -> Vec<Vec<ShapePrimitiveSource<'a>>> {
    let mut groups: Vec<Vec<ShapePrimitiveSource<'a>>> = Vec::new();
    let mut group_indices: HashMap<PanelShapeMergeKey, usize> = HashMap::new();
    for source in sources {
        let material = resolved_source_material(&source, context);
        let key = PanelShapeMergeKey::from_source(
            &source,
            material,
            effective_shape_shadow(&source, context),
        );
        if let Some(&index) = group_indices.get(&key) {
            groups[index].push(source);
        } else {
            group_indices.insert(key, groups.len());
            groups.push(vec![source]);
        }
    }
    groups
}

pub(super) fn reconcile_panel_line_batches(
    panels: Query<(
        Entity,
        &DiegeticPanel,
        &ComputedDiegeticPanel,
        &GlobalTransform,
        Option<&RenderLayers>,
        Option<&Visibility>,
        Option<&Resolved<Lighting>>,
        Option<&Resolved<Sidedness>>,
        Option<&Resolved<AntiAlias>>,
        Option<&Resolved<HairlineFade>>,
        Option<&Resolved<ShadowCasting>>,
        Option<&Resolved<ShapeMaterial>>,
        Option<&PanelShapes>,
    )>,
    shape_sources: Query<&PanelShapeSource>,
    mut removed_computed: RemovedComponents<ComputedDiegeticPanel>,
    mut removed_panels: RemovedComponents<DiegeticPanel>,
    lighting_default: Res<Lighting>,
    sidedness_default: Res<Sidedness>,
    anti_alias_default: Res<AntiAlias>,
    hairline_fade_default: Res<HairlineWidth>,
    material_assets: PanelShapeMaterialAssets,
    mut material_table: ResMut<FrameMaterialTableBuild>,
    mut store: ResMut<ShapeBatchStore>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<PathExtendedMaterial>>,
    mut storage_buffers: ResMut<Assets<ShaderBuffer>>,
    mut commands: Commands,
) {
    for panel in removed_computed.read().chain(removed_panels.read()) {
        store.remove_panel(panel);
        if let Ok(mut panel_commands) = commands.get_entity(panel) {
            panel_commands.despawn_related::<PanelShapes>();
        }
    }

    for (
        panel_entity,
        panel,
        computed,
        panel_transform,
        panel_layers,
        panel_visibility,
        panel_lighting,
        panel_sidedness,
        panel_anti_alias,
        panel_hairline_fade,
        panel_shadow_casting,
        panel_shape_material,
        panel_shapes,
    ) in &panels
    {
        let PanelLayout::Solved(result) = computed.layout() else {
            store.remove_panel(panel_entity);
            continue;
        };
        // Build material rows even while hidden so `Visibility` changes do not
        // renumber later users of the shared table. Only the batch records are
        // retired below.
        let hidden = is_hidden(panel_visibility);

        let (anchor_x, anchor_y) = panel.anchor_offsets();
        let context = PanelShapeReconcileContext {
            panel_entity,
            panel,
            standard_materials: &material_assets.standard_materials,
            shape_default: &material_assets.shape_default,
            shape_material: panel_shape_material.map_or_else(
                || material_assets.shape_default.0.clone(),
                |resolved| resolved.0.0.clone(),
            ),
            asset_server: &material_assets.asset_server,
            panel_transform: panel_transform.to_matrix(),
            path_context: PanelShapePathContext {
                points_to_world: panel.points_to_world(),
                anchor_x,
                anchor_y,
            },
            panel_shadow_casting: panel_shadow_casting
                .map_or(ShadowCasting::On, |resolved| resolved.0),
            layers: BatchRenderLayers(panel_layers.cloned().unwrap_or(RenderLayers::layer(0))),
            panel_lighting: panel_lighting.map_or(*lighting_default, |resolved| resolved.0),
            panel_sidedness: panel_sidedness.map_or(*sidedness_default, |resolved| resolved.0),
            panel_anti_alias: panel_anti_alias.map_or(*anti_alias_default, |resolved| resolved.0),
            panel_hairline_fade: panel_hairline_fade
                .map_or(hairline_fade_default.fade, |resolved| resolved.0),
        };
        let records = collect_panel_records(
            &context,
            &result.commands,
            computed.draw_order(),
            material_table.builder_mut(),
            &reconcile_panel_shape_sources(
                &mut commands,
                panel_entity,
                panel_shapes,
                &shape_sources,
                &result.commands,
            ),
            &material_assets.visual_overrides,
        );
        if hidden {
            store.remove_panel(panel_entity);
        } else {
            store.upsert_panel(panel_entity, records);
        }
    }

    store.rebuild_path_atlas_if_dirty();
    let atlas = store.commit_path_atlas(&mut storage_buffers, &mut materials);
    reconcile_batch_entities(
        atlas.as_ref(),
        *anti_alias_default,
        &mut store,
        &mut meshes,
        &mut materials,
        &mut storage_buffers,
        &mut commands,
    );
}

const fn is_hidden(visibility: Option<&Visibility>) -> bool {
    matches!(visibility, Some(Visibility::Hidden))
}

fn collect_panel_records(
    context: &PanelShapeReconcileContext<'_>,
    render_commands: &[RenderCommand],
    draw_order: &DrawOrder,
    material_table: &mut FrameMaterialTableBuilder,
    source_entities: &HashMap<PanelShapeSourceKey, Entity>,
    visual_overrides: &VisualOverrideIndex,
) -> Vec<(PathBatchKey, ShapeBatchRecord)> {
    group_line_primitives(
        collect_line_primitives(render_commands, draw_order, source_entities),
        context,
    )
    .into_iter()
    .filter_map(|group| build_panel_line_group(context, group, material_table, visual_overrides))
    .map(|built| (built.batch_key, built.record))
    .collect()
}

fn collect_line_primitives<'a>(
    render_commands: &'a [RenderCommand],
    draw_order: &DrawOrder,
    source_entities: &HashMap<PanelShapeSourceKey, Entity>,
) -> Vec<ShapePrimitiveSource<'a>> {
    let mut primitives = Vec::new();
    for (command_index, command) in render_commands.iter().enumerate() {
        if command.kind.draw_batch_family() != Some(DrawBatchFamily::PanelShape) {
            continue;
        }
        let RenderCommandKind::PanelShapes { shapes } = &command.kind else {
            continue;
        };
        let Some(draw_depth) = draw_order.depth_for(command_index) else {
            continue;
        };
        for line in shapes {
            if !source_entities.contains_key(&line.source_key()) {
                continue;
            }
            for primitive in line.primitives() {
                primitives.push(ShapePrimitiveSource {
                    element_index: command.element_idx,
                    draw_depth,
                    line,
                    primitive,
                });
            }
        }
    }
    primitives
}

fn reconcile_panel_shape_sources(
    commands: &mut Commands,
    panel_entity: Entity,
    panel_shapes: Option<&PanelShapes>,
    shape_sources: &Query<&PanelShapeSource>,
    render_commands: &[RenderCommand],
) -> HashMap<PanelShapeSourceKey, Entity> {
    let existing = collect_existing_shape_sources(panel_shapes, shape_sources);
    let mut live = HashMap::new();
    for line in resolved_panel_shapes(render_commands) {
        let key = line.source_key();
        if live.contains_key(&key) {
            continue;
        }
        let source = PanelShapeSource {
            key,
            command_index: line.source_command_index(),
        };
        let entity = if let Some(&entity) = existing.get(&key) {
            if shape_sources
                .get(entity)
                .is_ok_and(|current| *current != source)
                && let Ok(mut entity_commands) = commands.get_entity(entity)
            {
                entity_commands.insert(source);
            }
            entity
        } else {
            spawn_panel_shape_source(commands, panel_entity, source)
        };
        live.insert(key, entity);
    }
    for (key, entity) in existing {
        if !live.contains_key(&key)
            && let Ok(mut entity_commands) = commands.get_entity(entity)
        {
            entity_commands.despawn();
        }
    }
    live
}

fn collect_existing_shape_sources(
    panel_shapes: Option<&PanelShapes>,
    shape_sources: &Query<&PanelShapeSource>,
) -> HashMap<PanelShapeSourceKey, Entity> {
    let mut by_key = HashMap::new();
    let Some(panel_shapes) = panel_shapes else {
        return by_key;
    };
    for &entity in &**panel_shapes {
        if let Ok(source) = shape_sources.get(entity) {
            by_key.insert(source.key, entity);
        }
    }
    by_key
}

fn resolved_panel_shapes(
    render_commands: &[RenderCommand],
) -> impl Iterator<Item = &ResolvedPanelShape> {
    render_commands
        .iter()
        .flat_map(|command| match &command.kind {
            RenderCommandKind::PanelShapes { shapes } => shapes.as_slice(),
            _ => &[],
        })
}

fn spawn_panel_shape_source(
    commands: &mut Commands,
    panel_entity: Entity,
    source: PanelShapeSource,
) -> Entity {
    let mut spawned = Entity::PLACEHOLDER;
    commands.entity(panel_entity).with_children(|children| {
        spawned = children
            .spawn((
                PanelShape,
                source,
                PanelShapeOf(panel_entity),
                PanelOwned::from(panel_entity),
            ))
            .id();
    });
    spawned
}

fn build_panel_line_group(
    context: &PanelShapeReconcileContext<'_>,
    group: Vec<ShapePrimitiveSource<'_>>,
    material_table: &mut FrameMaterialTableBuilder,
    visual_overrides: &VisualOverrideIndex,
) -> Option<BuiltPanelShapePrimitive> {
    let members: Vec<&ShapePrimitiveSource<'_>> = group
        .iter()
        .filter(|source| !clipped_out(source.primitive.bounds(), source.primitive.clip()))
        .collect();
    let first = members.first()?;

    // Element override else the panel's cascade-resolved value; each line may
    // override both. Fade is per-curve in the merged path, so mixed policies
    // share this one group.
    let element_hairline_fade = context
        .panel
        .tree()
        .element_hairline_fade(first.element_index)
        .resolve(context.panel_hairline_fade);
    let path_members = panel_shape_path_members(&members, element_hairline_fade);
    let path = path::build_panel_shape_path(
        &path_members,
        first.line.owner_bounds(),
        first.primitive.clip(),
        &context.path_context,
    )?;
    // A merged silhouette has one depth; the lowest member offset keeps the
    // group at the depth the front-most authoring order produced alone.
    let clip_depth_nudge = members
        .iter()
        .map(|source| primitive_clip_depth_nudge(source))
        .fold(f32::INFINITY, f32::min);
    let oit_depth_offset = members
        .iter()
        .map(|source| primitive_oit_depth_offset(source))
        .fold(f32::INFINITY, f32::min);
    let slot_override = visual_overrides.get(context.panel_entity, first.element_index);
    let (fill_color, material_handle) = apply_shape_visual_override(
        slot_override,
        first.primitive.color(),
        resolved_source_material(first, context),
    );
    let base = render::material_asset_for_frame(
        context.standard_materials,
        context.asset_server,
        material_handle,
        &context.shape_default.0,
    )?;
    let base = strip_tangent_dependent_maps(base);
    let key = PanelShapeRenderKey {
        panel:  context.panel_entity,
        source: first.primitive.source_key(),
    };
    let alpha_mode = base.alpha_mode;
    let lighting = context.panel_lighting;
    let sidedness = context.panel_sidedness;
    let input = PanelShapeMaterialSlotInput {
        key,
        base_material: &base,
        fill_color,
        alpha_mode,
        lighting,
        sidedness,
    };
    let MaterialSlotAppend::Appended(appended) =
        material_table::append_material_slot(material_table, &input)
    else {
        return None;
    };
    let batch_key = PathBatchKey {
        z_index:                first.draw_depth.z_index(),
        z_index_rank:           first.draw_depth.z_index_rank(),
        batch_family:           DrawBatchFamily::PanelShape,
        shadow:                 effective_shape_shadow(first, context),
        layers:                 context.layers.clone(),
        pipeline_compatibility: appended.pipeline_compatibility,
        resource_compatibility: appended.resource_compatibility,
    };
    // Element override else the panel's cascade-resolved value. The merge key
    // groups per element, so every member of this group shares one resolution.
    let anti_alias = context
        .panel
        .tree()
        .element_anti_alias(first.element_index)
        .resolve(context.panel_anti_alias);
    let run = PathRenderRecord {
        transform: shape_run_transform(context.panel_transform, slot_override),
        material: appended.slot.into(),
        render_mode: u32::from(RenderMode::Text),
        clip_depth_nudge,
        oit_depth_offset,
        aa_flags: anti_alias.aa_flags(),
        text_coverage_bias: 0.0,
        text_em_height: 0.0,
    };
    let instance = path_quad_record(&path);
    Some(BuiltPanelShapePrimitive {
        batch_key,
        record: ShapeBatchRecord {
            key,
            draw_order_index: first.draw_depth.draw_order_index_value(),
            outline: path.outline,
            instance,
            run,
        },
    })
}

/// Builds the merged group's quad instance from its built analytic path; the
/// atlas rebuild stamps `packed_path_index` and `render_index` later.
const fn path_quad_record(path: &PanelShapePath) -> PathQuadRecord {
    PathQuadRecord {
        rect_min:          path.rect_min,
        rect_size:         path.rect_size,
        uv_min:            path.uv_min,
        uv_size:           path.uv_size,
        box_uv_min:        path.box_uv_min,
        box_uv_size:       path.box_uv_size,
        packed_path_index: 0,
        render_index:      0,
        box_uv_flip_x:     0,
    }
}

/// Applies one widget visual-slot override to a routed panel-line group.
///
/// Runs inside `build_panel_line_group` before the frame material row is
/// appended: a replacement `path_color` patches only the group's material-table
/// row values, while a replacement `material` that changes
/// `PipelineCompatibility`/`ResourceCompatibility` changes the computed
/// `PathBatchKey` and `ShapeBatchStore::upsert_panel` moves the group to its
/// compatible destination batch. A panel-local `offset` is composed into the
/// group's `PathRenderRecord` matrix by [`shape_run_transform`] instead.
fn apply_shape_visual_override<'a>(
    slot_override: Option<&'a VisualSlotOverride>,
    fill_color: Color,
    material: &'a Handle<StandardMaterial>,
) -> (Color, &'a Handle<StandardMaterial>) {
    let Some(slot_override) = slot_override else {
        return (fill_color, material);
    };
    (
        slot_override.path_color.unwrap_or(fill_color),
        slot_override.material.as_ref().unwrap_or(material),
    )
}

/// `PathRenderRecord` matrix for one panel-line group: the panel's global
/// matrix, right-multiplied by any slot-override `offset` as a panel-local
/// translation so a rotated or scaled panel moves the group in the panel
/// plane. The authored `PathOutline` contours and `PathQuadRecord` rect stay
/// untouched.
fn shape_run_transform(panel_transform: Mat4, slot_override: Option<&VisualSlotOverride>) -> Mat4 {
    slot_override
        .and_then(|slot_override| slot_override.offset)
        .map_or(panel_transform, |offset| {
            panel_transform * Mat4::from_translation(offset.extend(0.0))
        })
}

fn strip_tangent_dependent_maps(material: &StandardMaterial) -> StandardMaterial {
    let mut material = material.clone();
    material.normal_map_texture = None;
    material.depth_map = None;
    material
}

fn resolved_source_material<'a, 'source>(
    source: &'a ShapePrimitiveSource<'source>,
    context: &'a PanelShapeReconcileContext<'_>,
) -> &'a Handle<StandardMaterial>
where
    'source: 'a,
{
    source.primitive.material().map_or_else(
        || {
            context
                .panel
                .tree()
                .element_material(source.element_index)
                .map_or(&context.shape_material, core::convert::identity)
        },
        core::convert::identity,
    )
}

fn effective_shape_shadow(
    source: &ShapePrimitiveSource<'_>,
    context: &PanelShapeReconcileContext<'_>,
) -> VisualShadow {
    let element_shadow = context
        .panel
        .tree()
        .element_shadow_casting(source.element_index)
        .resolve(context.panel_shadow_casting);
    source.line.shadow_casting().resolve(element_shadow).into()
}

fn panel_shape_path_members<'a>(
    members: &[&ShapePrimitiveSource<'a>],
    element_hairline_fade: HairlineFade,
) -> Vec<PanelShapeMember<'a>> {
    members
        .iter()
        .map(|source| PanelShapeMember {
            primitive:     source.primitive,
            fade_exponent: source
                .line
                .hairline_fade()
                .resolve(element_hairline_fade)
                .fade_exponent(),
        })
        .collect()
}

fn primitive_clip_depth_nudge(source: &ShapePrimitiveSource<'_>) -> f32 {
    let line_depth = line_depth_order(source.line).to_f32().mul_add(
        PANEL_LINE_LINE_DEPTH_BIAS_STEP,
        source.draw_depth.clip_depth_nudge().get(),
    );
    source
        .primitive
        .part_order()
        .to_f32()
        .mul_add(PANEL_LINE_PART_DEPTH_BIAS_STEP, line_depth)
}

fn primitive_oit_depth_offset(source: &ShapePrimitiveSource<'_>) -> f32 {
    let line_depth = line_depth_order(source.line).to_f32().mul_add(
        PANEL_LINE_LINE_OIT_DEPTH_STEP,
        source.draw_depth.oit_depth_offset().get(),
    );
    source
        .primitive
        .part_order()
        .to_f32()
        .mul_add(PANEL_LINE_PART_OIT_DEPTH_STEP, line_depth)
}

const fn line_depth_order(line: &ResolvedPanelShape) -> usize {
    match line.source_key() {
        PanelShapeSourceKey::Element { line_ordinal, .. }
        | PanelShapeSourceKey::External { line_ordinal, .. } => line_ordinal,
    }
}

fn clipped_out(bounds: BoundingBox, clip: Option<BoundingBox>) -> bool {
    clip.is_some_and(|clip| bounds.intersect(&clip).is_none())
}

fn reconcile_batch_entities(
    atlas: Option<&PathAtlasHandles>,
    anti_alias: AntiAlias,
    store: &mut ShapeBatchStore,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PathExtendedMaterial>,
    storage_buffers: &mut Assets<ShaderBuffer>,
    commands: &mut Commands,
) {
    for entity in store.take_empty_batches() {
        commands.entity(entity).despawn();
    }
    let mut to_create = Vec::new();
    let mut to_grow = Vec::new();
    for (key, batch) in store.batches() {
        match &batch.gpu {
            None => to_create.push(key.clone()),
            Some(gpu)
                if batch.record_count() > gpu.capacity || batch.run_count() > gpu.run_capacity =>
            {
                to_grow.push(key.clone());
            },
            Some(_) => {},
        }
    }
    if let Some(atlas) = atlas {
        for key in to_create {
            spawn_batch_entity(
                &key,
                atlas,
                anti_alias,
                store,
                meshes,
                materials,
                storage_buffers,
                commands,
            );
        }
    }
    for key in to_grow {
        grow_batch_assets(&key, store, meshes, materials, storage_buffers, commands);
    }
    refresh_batch_material_depth_biases(store, materials);
}

fn spawn_batch_entity(
    key: &PathBatchKey,
    atlas: &PathAtlasHandles,
    anti_alias: AntiAlias,
    store: &mut ShapeBatchStore,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PathExtendedMaterial>,
    storage_buffers: &mut Assets<ShaderBuffer>,
    commands: &mut Commands,
) {
    let Some(batch) = store.get(key) else {
        return;
    };
    let capacity = batch.record_count().max(1).next_power_of_two();
    let run_capacity = batch.run_count().max(1).next_power_of_two();
    let instances = storage_buffers.add(ShaderBuffer::from(padded_line_instances(
        &batch.instances(),
        capacity,
    )));
    let run_table = storage_buffers.add(ShaderBuffer::from(padded_line_runs(
        &batch.run_records(),
        run_capacity,
    )));
    let mesh = meshes.add(inert_line_batch_mesh(capacity));
    let material = materials.add(line_batch_material(ShapeBatchMaterialInput {
        key,
        atlas,
        instances: instances.clone(),
        run_table: run_table.clone(),
        anti_alias,
    }));
    let mut batch_entity = commands.spawn((
        DiegeticPanelShapeBatch,
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material.clone()),
        Pickable::IGNORE,
        NoAutoAabb,
        Aabb::default(),
        key.layers.0.clone(),
    ));
    if key.shadow == VisualShadow::None {
        batch_entity.insert(NotShadowCaster);
    }
    let entity = batch_entity.id();

    if let Some(batch) = store.get_mut(key) {
        batch.entity = Some(entity);
        batch.gpu = Some(ShapeBatchGpu {
            instances,
            run_table,
            mesh,
            material,
            capacity,
            run_capacity,
        });
        batch.clear_render_record_dirty();
        batch.clear_path_quad_dirty();
    }
}

fn grow_batch_assets(
    key: &PathBatchKey,
    store: &mut ShapeBatchStore,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PathExtendedMaterial>,
    storage_buffers: &mut Assets<ShaderBuffer>,
    commands: &mut Commands,
) {
    let Some(batch) = store.get_mut(key) else {
        return;
    };
    let Some(entity) = batch.entity else {
        return;
    };
    let required = batch.record_count();
    let run_required = batch.run_count();
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

    let instances = storage_buffers.add(ShaderBuffer::from(padded_line_instances(
        &batch.instances(),
        capacity,
    )));
    let run_table = storage_buffers.add(ShaderBuffer::from(padded_line_runs(
        &batch.run_records(),
        run_capacity,
    )));
    let mesh = meshes.add(inert_line_batch_mesh(capacity));
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
    batch.clear_render_record_dirty();
    batch.clear_path_quad_dirty();
}

pub(super) fn update_panel_line_batch_bounds(
    mut store: ResMut<ShapeBatchStore>,
    mut batch_entities: Query<
        (&mut Transform, &mut GlobalTransform, &mut Aabb),
        With<DiegeticPanelShapeBatch>,
    >,
) {
    for (_, batch) in store.batches_mut() {
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
        *transform = Transform::from_translation(center);
        *global = GlobalTransform::from(*transform);
        *aabb = Aabb {
            center:       Vec3A::ZERO,
            half_extents: Vec3A::from((max - min) * 0.5),
        };
        batch.clear_bounds_dirty();
    }
}

pub(super) fn commit_panel_line_batch_buffers(
    mut store: ResMut<ShapeBatchStore>,
    mut staged_uploads: ResMut<RetainedBatchBufferUploads>,
    mut perf: ResMut<DiegeticPerfStats>,
) {
    let mut batches = 0_usize;
    let mut records = 0_usize;
    let mut uploads = 0_usize;
    perf.shape_breakdown.clear();
    for (key, batch) in store.batches_mut() {
        batches += 1;
        records += batch.record_count().to_usize();
        perf.shape_breakdown.push(render::batch_summary(
            key.z_index,
            &key.layers,
            key.shadow,
            &key.pipeline_compatibility,
            &key.resource_compatibility,
            batch.record_count(),
        ));
        if batch.gpu.is_none()
            || (!batch.path_quads_are_dirty() && !batch.render_records_are_dirty())
        {
            continue;
        }
        let capacity = batch.gpu.as_ref().map_or(0, |gpu| gpu.capacity);
        let run_capacity = batch.gpu.as_ref().map_or(0, |gpu| gpu.run_capacity);
        let instances = batch
            .path_quads_are_dirty()
            .then(|| padded_line_instances(&batch.instances(), capacity));
        let run_records = batch
            .render_records_are_dirty()
            .then(|| padded_line_runs(&batch.run_records(), run_capacity));
        batch.clear_path_quad_dirty();
        batch.clear_render_record_dirty();
        let Some(gpu) = &batch.gpu else {
            continue;
        };
        if let Some(instances) = instances {
            let handle = gpu.instances.clone();
            material_table::stage_retained_batch_buffer_upload(
                &mut staged_uploads,
                &handle,
                ShaderBuffer::from(instances),
            );
            uploads += 1;
        }
        if let Some(run_records) = run_records {
            let handle = gpu.run_table.clone();
            material_table::stage_retained_batch_buffer_upload(
                &mut staged_uploads,
                &handle,
                ShaderBuffer::from(run_records),
            );
            uploads += 1;
        }
    }
    perf.line_batch.batches = batches.into();
    perf.line_batch.records = records.into();
    perf.line_batch.uploads = uploads.into();
}

fn padded_line_instances(records: &[PathQuadRecord], capacity: u32) -> Vec<PathQuadRecord> {
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

fn padded_line_runs(records: &[PathRenderRecord], run_capacity: u32) -> Vec<PathRenderRecord> {
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

fn inert_line_batch_mesh(capacity: u32) -> Mesh {
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

struct ShapeBatchMaterialInput<'a> {
    key:        &'a PathBatchKey,
    atlas:      &'a PathAtlasHandles,
    instances:  Handle<ShaderBuffer>,
    run_table:  Handle<ShaderBuffer>,
    anti_alias: AntiAlias,
}

fn line_batch_material(input: ShapeBatchMaterialInput<'_>) -> PathExtendedMaterial {
    let ShapeBatchMaterialInput {
        key,
        atlas,
        instances,
        run_table,
        anti_alias,
    } = input;
    let base = StandardMaterial::default();
    let mut base = batch_key::apply_resource_compatibility_to_standard_material(
        &base,
        &key.resource_compatibility,
    );
    batch_key::apply_pipeline_compatibility_to_standard_material(
        &mut base,
        key.pipeline_compatibility,
    );
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
    store: &mut ShapeBatchStore,
    materials: &mut Assets<PathExtendedMaterial>,
) {
    for (key, batch) in store.batches_mut() {
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

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::f32::consts::FRAC_PI_2;
    use std::sync::Arc;

    use bevy::asset::AssetPlugin;
    use bevy::color::Color;
    use bevy::math::Vec4;
    use bevy::picking::hover::PickingInteraction;
    use bevy::prelude::App;
    use bevy::prelude::AssetApp;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::PostUpdate;
    use bevy::prelude::Quat;
    use bevy::prelude::RelationshipTarget;
    use bevy::prelude::Update;

    use super::*;
    use crate::Appearance;
    use crate::CalloutCap;
    use crate::El;
    use crate::Mm;
    use crate::PanelWidget;
    use crate::cascade;
    use crate::cascade::Cascade;
    use crate::cascade::CascadeSet;
    use crate::cascade::ShapeMaterial;
    use crate::layout::DrawZIndex;
    use crate::layout::LayoutBuilder;
    use crate::layout::LayoutTree;
    use crate::layout::PanelDraw;
    use crate::layout::PanelLine;
    use crate::layout::PanelShapePrimitiveGeometry;
    use crate::layout::PanelShapePrimitiveKey;
    use crate::layout::PanelShapePrimitiveKind;
    use crate::layout::Text;
    use crate::layout::TextDimensions;
    use crate::layout::TextMeasure;
    use crate::layout::TextStyle;
    use crate::panel::DiegeticPanelCommands;
    use crate::panel::HeadlessLayoutPlugin;
    use crate::render::BatchAlphaMode;
    use crate::render::Bounds;
    use crate::render::HairlineWidth;
    use crate::render::PathContour;
    use crate::render::QuadraticSegment;
    use crate::render::draw_order::DrawZIndexRank;
    use crate::render::material_table::MaterialSlotValues;
    use crate::render::material_table::MaterialTableAppendReady;
    use crate::render::material_table::MaterialTablePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::VisualElementCapabilities;
    use crate::widgets::VisualSlotId;
    use crate::widgets::VisualSlotOverride;
    use crate::widgets::WidgetVisualOverrides;
    use crate::widgets::WidgetVisualSlots;
    use crate::widgets::WidgetsPlugin;

    const SHAPE_OVERRIDE_COLOR: Color = Color::srgb(0.2, 0.8, 0.4);
    const SHAPE_SLOT: VisualSlotId = VisualSlotId::new(4);
    /// Panel-local offset with distinct axis values so a rotated panel maps
    /// it to a visibly different world delta.
    const SHAPE_SLOT_OFFSET: Vec2 = Vec2::new(3.0, 1.5);

    /// Headless app wired with panel layout, the AA/fade cascade plugins
    /// (via [`HeadlessLayoutPlugin`]), the lighting/sidedness cascade plugins
    /// (which `RenderPlugin` gets from `TextRenderPlugin`), the production
    /// anti-alias material sync, and the panel-line reconcile.
    fn line_batch_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .add_plugins(MaterialTablePlugin)
            .insert_resource(DiegeticTextMeasurer {
                measure_fn: Arc::new(|_text: &str, measure: &TextMeasure| TextDimensions {
                    width:       measure.size,
                    height:      measure.size,
                    line_height: measure.size,
                }),
            })
            .add_plugins(HeadlessLayoutPlugin)
            .add_plugins(cascade::cascade_plugin::<ShapeMaterial>())
            .add_plugins(cascade::cascade_plugin::<Lighting>())
            .add_plugins(cascade::cascade_plugin::<Sidedness>())
            .init_resource::<AntiAlias>()
            .init_resource::<HairlineWidth>()
            .init_resource::<ShapeBatchStore>()
            .init_resource::<VisualOverrideIndex>()
            .init_asset::<Mesh>()
            .init_asset::<PathExtendedMaterial>()
            .init_asset::<ShaderBuffer>()
            .add_systems(
                Update,
                crate::render::sync_anti_alias.before(CascadeSet::Propagate),
            )
            .add_systems(
                PostUpdate,
                reconcile_panel_line_batches.after(MaterialTableAppendReady),
            );
        render::seed_default_material_cascades(&mut app);
        app
    }

    fn horizontal_line() -> PanelLine {
        PanelLine::new((0.0, 5.0), (20.0, 5.0))
            .width(0.4)
            .color(Color::WHITE)
    }

    fn horizontal_line_with_color(color: Color) -> PanelLine {
        PanelLine::new((0.0, 5.0), (20.0, 5.0))
            .width(0.4)
            .color(color)
    }

    fn horizontal_line_with_material(material: Handle<StandardMaterial>) -> PanelLine {
        horizontal_line().material(material)
    }

    fn one_line_tree(line: PanelLine) -> LayoutTree {
        LayoutBuilder::with_root(El::new().size(40.0, 20.0).draw(PanelDraw::lines([line]))).build()
    }

    fn two_line_tree(first: PanelLine, second: PanelLine) -> LayoutTree {
        LayoutBuilder::with_root(
            El::new()
                .size(40.0, 20.0)
                .draw(PanelDraw::lines([first, second])),
        )
        .build()
    }

    fn material_base_color(color: Color) -> Vec4 {
        let linear = color.to_linear();
        Vec4::new(linear.red, linear.green, linear.blue, linear.alpha)
    }

    fn material_with_metallic(app: &mut App, metallic: f32) -> Handle<StandardMaterial> {
        app.world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                metallic,
                ..Default::default()
            })
    }

    fn material_asset(app: &mut App, material: StandardMaterial) -> Handle<StandardMaterial> {
        app.world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(material)
    }

    fn spawn_line_panel(app: &mut App, z_index: impl Into<DrawZIndex>) -> Entity {
        let z_index = z_index.into();
        let line_element = El::new()
            .size(40.0, 20.0)
            .draw(PanelDraw::lines([horizontal_line()]))
            .z_index(z_index);
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(60.0))
            .layout(|builder| {
                builder.with(line_element, |_| {});
            })
            .build()
            .unwrap_or_else(|error| panic!("line panel should build: {error:?}"));
        app.world_mut().spawn(panel).id()
    }

    fn settle(app: &mut App) {
        for _ in 0..3 {
            app.update();
        }
    }

    fn one_line_batch_values(app: &App) -> (DrawZIndex, f32, f32, Vec<(f32, f32)>) {
        let store = app.world().resource::<ShapeBatchStore>();
        let Some((key, batch)) = store.batches().next() else {
            panic!("one line batch should exist");
        };
        let Some(gpu) = batch.gpu.as_ref() else {
            panic!("line batch should have GPU assets");
        };
        let Some(material) = app
            .world()
            .resource::<Assets<PathExtendedMaterial>>()
            .get(&gpu.material)
        else {
            panic!("line batch material should exist");
        };
        let mut records: Vec<(f32, f32)> = batch
            .run_records()
            .into_iter()
            .map(|record| (record.clip_depth_nudge, record.oit_depth_offset))
            .collect();
        records.sort_by(|left, right| left.0.total_cmp(&right.0));
        (
            key.z_index,
            material.base.depth_bias,
            render::path_material_oit_depth_offset(material),
            records,
        )
    }

    fn first_shape_material_values(app: &App) -> MaterialSlotValues {
        let store = app.world().resource::<ShapeBatchStore>();
        let Some((_, batch)) = store.batches().next() else {
            panic!("one line batch should exist");
        };
        let records = batch.run_records();
        let Some(record) = records.first() else {
            panic!("line batch should have a run record");
        };
        let table = app
            .world()
            .resource::<FrameMaterialTableBuild>()
            .table()
            .rows();
        table[record.material.as_u32().to_usize()]
    }

    fn first_line_batch_material_values(app: &App) -> Vec<MaterialSlotValues> {
        let store = app.world().resource::<ShapeBatchStore>();
        let Some((_, batch)) = store.batches().next() else {
            panic!("one line batch should exist");
        };
        let table = app
            .world()
            .resource::<FrameMaterialTableBuild>()
            .table()
            .rows();
        batch
            .run_records()
            .iter()
            .map(|record| table[record.material.as_u32().to_usize()])
            .collect()
    }

    fn panel_shape_sources(app: &App, panel: Entity) -> Vec<Entity> {
        app.world()
            .get::<PanelShapes>(panel)
            .map(|sources| sources.iter().collect())
            .unwrap_or_default()
    }

    /// Per record: the run's AA flags and the packed outline's fade exponent
    /// (fade is per-curve data carried by the record's contours, not a run
    /// field).
    fn sorted_run_fields(store: &ShapeBatchStore) -> Vec<(u32, u32)> {
        let mut fields: Vec<(u32, u32)> = store
            .batches()
            .flat_map(|(_, batch)| &batch.records)
            .map(|record| {
                (
                    record.run.aa_flags,
                    record.outline.contours[0].fade_exponent.to_bits(),
                )
            })
            .collect();
        fields.sort_unstable();
        fields
    }

    #[test]
    fn same_silhouette_different_colors_keep_distinct_records() {
        let mut app = line_batch_app();
        let red = Color::srgb(1.0, 0.0, 0.0);
        let blue = Color::srgb(0.0, 0.0, 1.0);
        let line_element = El::new().size(40.0, 20.0).draw(PanelDraw::lines([
            horizontal_line_with_color(red),
            horizontal_line_with_color(blue),
        ]));
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(60.0))
            .layout(|builder| {
                builder.with(line_element, |_| {});
            })
            .build()
            .unwrap_or_else(|error| panic!("line panel should build: {error:?}"));
        app.world_mut().spawn(panel);
        settle(&mut app);

        let store = app.world().resource::<ShapeBatchStore>();
        assert_eq!(store.batches().count(), 1);
        let Some((_, batch)) = store.batches().next() else {
            panic!("one line batch should exist");
        };
        assert_eq!(batch.record_count(), 2);

        let table = app
            .world()
            .resource::<FrameMaterialTableBuild>()
            .table()
            .rows();
        assert_eq!(table.len(), 2);
        let colors: Vec<Vec4> = batch
            .run_records()
            .iter()
            .map(|record| table[record.material.as_u32().to_usize()].base_color)
            .collect();
        assert_eq!(colors.len(), 2);
        assert!(colors.contains(&material_base_color(red)));
        assert!(colors.contains(&material_base_color(blue)));
    }

    #[test]
    fn same_colored_end_cap_shares_material_row() {
        let mut app = line_batch_app();
        let color = Color::srgb(0.9, 0.1, 0.1);
        let line = horizontal_line_with_color(color).end_cap(CalloutCap::circle().color(color));
        app.world_mut().spawn(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(60.0))
                .with_tree(one_line_tree(line))
                .build()
                .unwrap_or_else(|error| panic!("line panel should build: {error:?}")),
        );
        settle(&mut app);

        let store = app.world().resource::<ShapeBatchStore>();
        assert_eq!(store.batches().count(), 1);
        let Some((_, batch)) = store.batches().next() else {
            panic!("one line batch should exist");
        };
        assert_eq!(batch.record_count(), 1);
        let rows = first_line_batch_material_values(&app);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].base_color, material_base_color(color));
    }

    #[test]
    fn differently_colored_end_cap_gets_distinct_material_row() {
        let mut app = line_batch_app();
        let line_color = Color::srgb(0.9, 0.1, 0.1);
        let cap_color = Color::srgb(0.1, 0.1, 0.9);
        let line =
            horizontal_line_with_color(line_color).end_cap(CalloutCap::circle().color(cap_color));
        app.world_mut().spawn(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(60.0))
                .with_tree(one_line_tree(line))
                .build()
                .unwrap_or_else(|error| panic!("line panel should build: {error:?}")),
        );
        settle(&mut app);

        let store = app.world().resource::<ShapeBatchStore>();
        assert_eq!(store.batches().count(), 1);
        let Some((_, batch)) = store.batches().next() else {
            panic!("one line batch should exist");
        };
        assert_eq!(batch.record_count(), 2);
        let colors: Vec<Vec4> = first_line_batch_material_values(&app)
            .iter()
            .map(|values| values.base_color)
            .collect();
        assert_eq!(colors.len(), 2);
        assert!(colors.contains(&material_base_color(line_color)));
        assert!(colors.contains(&material_base_color(cap_color)));
    }

    #[test]
    fn shape_source_entities_reuse_across_material_only_edits() {
        let mut app = line_batch_app();
        let red = Color::srgb(1.0, 0.0, 0.0);
        let blue = Color::srgb(0.0, 0.0, 1.0);
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(60.0))
                    .with_tree(one_line_tree(horizontal_line_with_color(red)))
                    .build()
                    .unwrap_or_else(|error| panic!("line panel should build: {error:?}")),
            )
            .id();
        settle(&mut app);
        let before = panel_shape_sources(&app, panel);
        assert_eq!(before.len(), 1);
        assert_eq!(
            first_shape_material_values(&app).base_color,
            material_base_color(red)
        );

        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, one_line_tree(horizontal_line_with_color(blue)))
                .is_ok()
        );
        settle(&mut app);

        assert_eq!(panel_shape_sources(&app, panel), before);
        assert_eq!(
            first_shape_material_values(&app).base_color,
            material_base_color(blue)
        );
        let store = app.world().resource::<ShapeBatchStore>();
        let Some((_, batch)) = store.batches().next() else {
            panic!("one line batch should exist");
        };
        assert_eq!(batch.record_count(), 1);
        assert!(!batch.path_quads_are_dirty());
    }

    #[test]
    fn removed_panel_shape_source_despawns_relationship_entity() {
        let mut app = line_batch_app();
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(60.0))
                    .with_tree(one_line_tree(horizontal_line()))
                    .build()
                    .unwrap_or_else(|error| panic!("line panel should build: {error:?}")),
            )
            .id();
        settle(&mut app);
        let before = panel_shape_sources(&app, panel);
        assert_eq!(before.len(), 1);

        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, LayoutBuilder::new(100.0, 50.0).build())
                .is_ok()
        );
        settle(&mut app);

        assert!(panel_shape_sources(&app, panel).is_empty());
        assert!(app.world().get_entity(before[0]).is_err());
    }

    #[test]
    fn shape_run_inherits_panel_shape_material_through_cascade() {
        let mut app = line_batch_app();
        let panel_material = material_with_metallic(&mut app, 0.46);
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(60.0))
            .shape_material(panel_material)
            .layout(|builder| {
                builder.with(
                    El::new()
                        .size(40.0, 20.0)
                        .draw(PanelDraw::lines([horizontal_line()])),
                    |_| {},
                );
            })
            .build()
            .unwrap_or_else(|error| panic!("line panel should build: {error:?}"));
        app.world_mut().spawn(panel);
        settle(&mut app);

        assert_eq!(
            first_shape_material_values(&app).metallic.to_bits(),
            0.46_f32.to_bits()
        );
    }

    #[test]
    fn element_material_overrides_panel_shape_material_for_shape_primitives() {
        let mut app = line_batch_app();
        let panel_shape_material = material_with_metallic(&mut app, 0.35);
        let element_material = material_with_metallic(&mut app, 0.64);
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(60.0))
            .shape_material(panel_shape_material)
            .layout(|builder| {
                builder.with(
                    El::new()
                        .size(40.0, 20.0)
                        .material(element_material)
                        .draw(PanelDraw::lines([horizontal_line()])),
                    |_| {},
                );
            })
            .build()
            .unwrap_or_else(|error| panic!("line panel should build: {error:?}"));
        app.world_mut().spawn(panel);
        settle(&mut app);

        assert_eq!(
            first_shape_material_values(&app).metallic.to_bits(),
            0.64_f32.to_bits()
        );
    }

    #[test]
    fn panel_surface_material_does_not_feed_shape_materials() {
        let mut app = line_batch_app();
        let panel_material = material_with_metallic(&mut app, 0.57);
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(60.0))
            .material(panel_material)
            .layout(|builder| {
                builder.with(
                    El::new()
                        .size(40.0, 20.0)
                        .draw(PanelDraw::lines([horizontal_line()])),
                    |_| {},
                );
            })
            .build()
            .unwrap_or_else(|error| panic!("line panel should build: {error:?}"));
        app.world_mut().spawn(panel);
        settle(&mut app);

        assert_eq!(
            first_shape_material_values(&app).metallic.to_bits(),
            render::default_panel_material().metallic.to_bits()
        );
    }

    #[test]
    fn shape_local_material_wins_over_panel_materials() {
        let mut app = line_batch_app();
        let panel_material = material_with_metallic(&mut app, 0.21);
        let panel_shape_material = material_with_metallic(&mut app, 0.35);
        let local_material = material_with_metallic(&mut app, 0.93);
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(60.0))
            .material(panel_material)
            .shape_material(panel_shape_material)
            .layout(|builder| {
                builder.with(
                    El::new().size(40.0, 20.0).draw(PanelDraw::lines([
                        horizontal_line_with_material(local_material),
                    ])),
                    |_| {},
                );
            })
            .build()
            .unwrap_or_else(|error| panic!("line panel should build: {error:?}"));
        app.world_mut().spawn(panel);
        settle(&mut app);

        assert_eq!(
            first_shape_material_values(&app).metallic.to_bits(),
            0.93_f32.to_bits()
        );
    }

    #[test]
    fn scalar_distinct_shape_material_sources_share_one_batch_with_distinct_rows() {
        let mut app = line_batch_app();
        let first_material = material_asset(
            &mut app,
            StandardMaterial {
                metallic: 0.18,
                alpha_mode: AlphaMode::Blend,
                ..Default::default()
            },
        );
        let second_material = material_asset(
            &mut app,
            StandardMaterial {
                metallic: 0.82,
                alpha_mode: AlphaMode::Blend,
                ..Default::default()
            },
        );
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(60.0))
            .with_tree(two_line_tree(
                horizontal_line_with_material(first_material),
                horizontal_line_with_material(second_material),
            ))
            .build()
            .unwrap_or_else(|error| panic!("line panel should build: {error:?}"));
        app.world_mut().spawn(panel);
        settle(&mut app);

        let store = app.world().resource::<ShapeBatchStore>();
        assert_eq!(store.batches().count(), 1);
        let Some((_, batch)) = store.batches().next() else {
            panic!("one line batch should exist");
        };
        assert_eq!(batch.record_count(), 2);
        let table = app
            .world()
            .resource::<FrameMaterialTableBuild>()
            .table()
            .rows();
        let metallic: Vec<u32> = batch
            .run_records()
            .iter()
            .map(|record| {
                table[record.material.as_u32().to_usize()]
                    .metallic
                    .to_bits()
            })
            .collect();
        assert!(metallic.contains(&0.18_f32.to_bits()));
        assert!(metallic.contains(&0.82_f32.to_bits()));
    }

    #[test]
    fn alpha_or_texture_shape_material_splitters_create_separate_batches() {
        let mut alpha_app = line_batch_app();
        let blend = material_asset(
            &mut alpha_app,
            StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                ..Default::default()
            },
        );
        let add = material_asset(
            &mut alpha_app,
            StandardMaterial {
                alpha_mode: AlphaMode::Add,
                ..Default::default()
            },
        );
        alpha_app.world_mut().spawn(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(60.0))
                .with_tree(two_line_tree(
                    horizontal_line_with_material(blend),
                    horizontal_line_with_material(add),
                ))
                .build()
                .unwrap_or_else(|error| panic!("line panel should build: {error:?}")),
        );
        settle(&mut alpha_app);
        assert_eq!(
            alpha_app
                .world()
                .resource::<ShapeBatchStore>()
                .batches()
                .count(),
            2
        );

        let mut texture_app = line_batch_app();
        let untextured = material_asset(
            &mut texture_app,
            StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                ..Default::default()
            },
        );
        let textured = material_asset(
            &mut texture_app,
            StandardMaterial {
                base_color_texture: Some(Handle::default()),
                alpha_mode: AlphaMode::Blend,
                ..Default::default()
            },
        );
        texture_app.world_mut().spawn(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(60.0))
                .with_tree(two_line_tree(
                    horizontal_line_with_material(untextured),
                    horizontal_line_with_material(textured),
                ))
                .build()
                .unwrap_or_else(|error| panic!("line panel should build: {error:?}")),
        );
        settle(&mut texture_app);
        let store = texture_app.world().resource::<ShapeBatchStore>();
        assert_eq!(store.batches().count(), 2);
        assert!(
            store
                .batches()
                .any(|(key, _)| key.resource_compatibility.base_color_texture.is_some())
        );
    }

    #[test]
    fn default_lines_across_panels_share_one_level_zero_batch() {
        let mut app = line_batch_app();
        spawn_line_panel(&mut app, DrawZIndex::default());
        spawn_line_panel(&mut app, DrawZIndex::default());
        settle(&mut app);

        let store = app.world().resource::<ShapeBatchStore>();
        assert_eq!(store.batches().count(), 1);
        let Some((_, batch)) = store.batches().next() else {
            panic!("one line batch should exist");
        };
        assert_eq!(batch.record_count(), 2);

        let (z_index, material_depth_bias, material_oit_offset, records) =
            one_line_batch_values(&app);
        assert_eq!(z_index, DrawZIndex::default());
        assert_eq!(material_depth_bias.to_bits(), 0.0_f32.to_bits());
        assert_eq!(material_oit_offset.to_bits(), 0.0_f32.to_bits());
        assert_eq!(
            records,
            vec![
                (0.0, crate::render::constants::OIT_DEPTH_STEP),
                (0.0, crate::render::constants::OIT_DEPTH_STEP),
            ],
            "per-record offsets stay in the run table"
        );
    }

    #[test]
    fn line_z_indexes_route_to_matching_level_batches() {
        let mut app = line_batch_app();
        spawn_line_panel(&mut app, -1);
        spawn_line_panel(&mut app, 1);
        settle(&mut app);

        let store = app.world().resource::<ShapeBatchStore>();
        let mut z_indices: Vec<DrawZIndex> = store.batches().map(|(key, _)| key.z_index).collect();
        z_indices.sort_unstable();
        assert_eq!(z_indices, vec![DrawZIndex(-1), DrawZIndex(1)]);

        let materials = app.world().resource::<Assets<PathExtendedMaterial>>();
        let mut depth_biases: Vec<(DrawZIndex, u32)> = store
            .batches()
            .map(|(key, batch)| {
                let Some(gpu) = batch.gpu.as_ref() else {
                    panic!("line batch should have GPU assets");
                };
                let Some(material) = materials.get(&gpu.material) else {
                    panic!("line batch material should exist");
                };
                (key.z_index, material.base.depth_bias.to_bits())
            })
            .collect();
        depth_biases.sort_by_key(|(z_index, _)| *z_index);
        assert_eq!(
            depth_biases,
            vec![
                (DrawZIndex(-1), 0.0_f32.to_bits()),
                (DrawZIndex(1), 0.0_f32.to_bits()),
            ],
        );
    }

    /// An element-level AA override renders with its own
    /// mode while its sibling keeps the inherited mode without increasing the
    /// batch count, and a global `AntiAlias` / `HairlineWidth::fade`
    /// change applied after the override exists re-packs the non-overridden
    /// run records while the overrides hold.
    #[test]
    fn element_overrides_share_one_batch_and_global_changes_repack() {
        let mut app = line_batch_app();

        // Element 1: no overrides. Element 2: AA Off + fade pinned to Full.
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(60.0))
            .layout(|builder| {
                builder.with(
                    El::new()
                        .size(40.0, 20.0)
                        .draw(PanelDraw::lines([horizontal_line()])),
                    |_| {},
                );
                builder.with(
                    El::new()
                        .size(40.0, 20.0)
                        .anti_alias(AntiAlias::Off)
                        .hairline_fade(HairlineFade::Full)
                        .draw(PanelDraw::lines([horizontal_line()])),
                    |_| {},
                );
            })
            .build()
            .unwrap_or_else(|error| panic!("test panel should build: {error:?}"));
        app.world_mut().spawn(panel);
        for _ in 0..3 {
            app.update();
        }

        {
            let store = app.world().resource::<ShapeBatchStore>();
            assert_eq!(
                store.batches().count(),
                1,
                "an element AA override must not split the batch"
            );
            assert_eq!(
                sorted_run_fields(store),
                vec![
                    (AntiAlias::Off.aa_flags(), 0.0_f32.to_bits()),
                    (AntiAlias::Both.aa_flags(), 0.0_f32.to_bits()),
                ],
                "one run carries the element override, the other the global default"
            );
        }

        // Global changes after the overrides exist: the cascade-root sync +
        // propagation re-resolve the panel, and `Changed<Resolved<A>>`
        // re-packs the run records.
        *app.world_mut().resource_mut::<AntiAlias>() = AntiAlias::Anisotropic;
        app.world_mut().resource_mut::<HairlineWidth>().fade = HairlineFade::Fade { exponent: 1.5 };
        for _ in 0..3 {
            app.update();
        }

        let store = app.world().resource::<ShapeBatchStore>();
        assert_eq!(
            sorted_run_fields(store),
            vec![
                (AntiAlias::Off.aa_flags(), 0.0_f32.to_bits()),
                (AntiAlias::Anisotropic.aa_flags(), 1.5_f32.to_bits()),
            ],
            "the non-overridden run re-packs to the new globals; the element overrides hold"
        );
    }

    /// The typography overlay rebuilds its guide panels
    /// by despawning and respawning them on every metric refresh, so the
    /// panel-entity portion of every batch key churns. Repeated recreation
    /// must leave zero records keyed to a dead panel entity.
    #[test]
    fn recreated_guide_panels_leave_no_stale_records() {
        fn spawn_guide_panel(app: &mut App, offset: f32) -> Entity {
            let panel = DiegeticPanel::world()
                .size(Mm(100.0), Mm(60.0))
                .layout(move |builder| {
                    builder.with(
                        El::new()
                            .size(40.0, 20.0)
                            .hairline_fade(HairlineFade::Full)
                            .draw(PanelDraw::lines([
                                PanelLine::new((0.0, offset), (20.0, offset))
                                    .width(0.4)
                                    .color(Color::WHITE),
                                PanelLine::new((offset, 0.0), (offset, 15.0))
                                    .width(0.4)
                                    .color(Color::WHITE),
                            ])),
                        |_| {},
                    );
                })
                .build()
                .unwrap_or_else(|error| panic!("guide panel should build: {error:?}"));
            app.world_mut().spawn(panel).id()
        }

        let mut app = line_batch_app();
        let mut current = spawn_guide_panel(&mut app, 5.0);
        for _ in 0..3 {
            app.update();
        }

        for refresh in 0..5 {
            app.world_mut().entity_mut(current).despawn();
            current = spawn_guide_panel(&mut app, 4.0 + refresh.to_f32());
            for _ in 0..3 {
                app.update();
            }

            let store = app.world().resource::<ShapeBatchStore>();
            let stale_records: Vec<Entity> = store
                .batches()
                .flat_map(|(_, batch)| &batch.records)
                .map(|record| record.key.panel)
                .filter(|panel| *panel != current)
                .collect();
            assert!(
                stale_records.is_empty(),
                "refresh {refresh}: records keyed to dead panels: {stale_records:?}"
            );

            let stale_index: Vec<Entity> = store
                .panel_members
                .keys()
                .copied()
                .filter(|panel| *panel != current)
                .collect();
            assert!(
                stale_index.is_empty(),
                "refresh {refresh}: panel index holds dead panels: {stale_index:?}"
            );
        }

        let store = app.world().resource::<ShapeBatchStore>();
        let record_count: usize = store.batches().map(|(_, batch)| batch.records.len()).sum();
        assert_eq!(
            record_count, 1,
            "the surviving panel's guide lines must stay batched as one merged record"
        );
    }

    #[test]
    fn line_ordinal_contributes_to_depth_inside_grouped_command() {
        let first = test_line(0);
        let second = test_line(1);
        let command_bounds = first.visual_bounds();
        let commands = vec![RenderCommand {
            bounds:      command_bounds,
            element_idx: 0,
            kind:        RenderCommandKind::PanelShapes {
                shapes: vec![first.clone(), second.clone()],
            },
            z_index:     DrawZIndex::default(),
        }];
        let draw_depth = draw_depth_for_command(&commands, 0);
        let first_source = ShapePrimitiveSource {
            element_index: 0,
            draw_depth,
            line: &first,
            primitive: &first.primitives()[0],
        };
        let second_source = ShapePrimitiveSource {
            element_index: 0,
            draw_depth,
            line: &second,
            primitive: &second.primitives()[0],
        };

        assert!(
            primitive_clip_depth_nudge(&second_source) > primitive_clip_depth_nudge(&first_source)
        );
        assert!(
            primitive_oit_depth_offset(&second_source) > primitive_oit_depth_offset(&first_source)
        );
    }

    #[test]
    fn collect_line_primitives_preserves_resolved_line_clip() {
        let inherited_clip = BoundingBox {
            x:      0.0,
            y:      0.0,
            width:  100.0,
            height: 100.0,
        };
        let owner_clip = BoundingBox {
            x:      10.0,
            y:      10.0,
            width:  10.0,
            height: 10.0,
        };
        let source_key = PanelShapeSourceKey::element(1, 0, 0);
        let primitive = ResolvedPanelShapePrimitive {
            source_key: PanelShapePrimitiveKey::new(source_key, 0),
            kind:       PanelShapePrimitiveKind::Segment,
            geometry:   PanelShapePrimitiveGeometry::Segment {
                start: Vec2::new(0.0, 0.0),
                end:   Vec2::new(50.0, 0.0),
                width: 2.0,
            },
            color:      Color::WHITE,
            material:   Cascade::Inherit,
            bounds:     BoundingBox {
                x:      0.0,
                y:      -1.0,
                width:  50.0,
                height: 2.0,
            },
            clip:       Some(inherited_clip),
            part_order: 0,
        };
        let primitive_bounds = primitive.bounds();
        let line = ResolvedPanelShape {
            source_key,
            source_command_index: 0,
            owner_bounds: owner_clip,
            visual_bounds: primitive_bounds,
            clip: Some(inherited_clip),
            start: Vec2::ZERO,
            end: Vec2::new(50.0, 0.0),
            shaft_start: Vec2::ZERO,
            shaft_end: Vec2::new(50.0, 0.0),
            width: 2.0,
            color: Color::WHITE,
            material: Cascade::Inherit,
            hairline_fade: Cascade::Inherit,
            shadow_casting: Cascade::Inherit,
            primitives: vec![primitive],
        };
        let commands = vec![RenderCommand {
            bounds:      primitive_bounds,
            element_idx: 1,
            kind:        RenderCommandKind::PanelShapes { shapes: vec![line] },
            z_index:     DrawZIndex::default(),
        }];

        let draw_order = DrawOrder::from_commands(&commands);
        let mut source_entities = HashMap::new();
        source_entities.insert(source_key, Entity::from_bits(10));
        let collected = collect_line_primitives(&commands, &draw_order, &source_entities);

        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].primitive.clip(), Some(inherited_clip));
    }

    #[test]
    fn two_panels_with_same_key_share_one_line_batch() {
        let mut store = ShapeBatchStore::default();
        let key = test_batch_key();
        let first_panel = Entity::from_bits(1);
        let second_panel = Entity::from_bits(2);

        store.upsert_panel(
            first_panel,
            vec![(key.clone(), test_batch_record(first_panel, 0))],
        );
        store.upsert_panel(
            second_panel,
            vec![(key.clone(), test_batch_record(second_panel, 1))],
        );

        assert_eq!(store.batches().count(), 1);
        assert_eq!(store.get(&key).map(ShapeBatch::record_count), Some(2));
    }

    #[test]
    fn panel_shape_batch_entities_ignore_stock_mesh_picking() {
        let mut app = line_batch_app();
        spawn_line_panel(&mut app, DrawZIndex::default());
        settle(&mut app);

        let mut query = app
            .world_mut()
            .query_filtered::<(Entity, &Pickable), With<DiegeticPanelShapeBatch>>();
        let batches = query.iter(app.world()).collect::<Vec<_>>();
        assert_eq!(batches.len(), 1);
        assert!(!batches[0].1.is_hoverable);
        assert!(!batches[0].1.should_block_lower);
    }

    #[test]
    fn removing_a_panel_removes_only_its_line_records() {
        let mut store = ShapeBatchStore::default();
        let key = test_batch_key();
        let first_panel = Entity::from_bits(1);
        let second_panel = Entity::from_bits(2);

        store.upsert_panel(
            first_panel,
            vec![(key.clone(), test_batch_record(first_panel, 0))],
        );
        store.upsert_panel(
            second_panel,
            vec![(key.clone(), test_batch_record(second_panel, 1))],
        );

        store.remove_panel(first_panel);

        assert_eq!(store.get(&key).map(ShapeBatch::record_count), Some(1));
    }

    #[test]
    fn batch_key_change_preserves_unmoved_line_record() {
        let mut store = ShapeBatchStore::default();
        let first_key = test_batch_key();
        let mut second_key = test_batch_key();
        second_key.z_index_rank = DrawZIndexRank::from(1);
        let panel = Entity::from_bits(1);
        let stable_member = test_batch_record(panel, 0).key;

        store.upsert_panel(
            panel,
            vec![
                (first_key.clone(), test_batch_record(panel, 0)),
                (first_key.clone(), test_batch_record(panel, 1)),
            ],
        );
        {
            let Some(batch) = store.get_mut(&first_key) else {
                panic!("first batch should exist");
            };
            let Some(record) = batch
                .records
                .iter_mut()
                .find(|record| record.key == stable_member)
            else {
                panic!("stable member should exist before the key change");
            };
            record.instance.packed_path_index = u32::MAX;
        }

        store.upsert_panel(
            panel,
            vec![
                (first_key.clone(), test_batch_record(panel, 0)),
                (second_key.clone(), test_batch_record(panel, 1)),
            ],
        );

        assert_eq!(store.get(&first_key).map(ShapeBatch::record_count), Some(1));
        assert_eq!(
            store.get(&second_key).map(ShapeBatch::record_count),
            Some(1)
        );
        let Some(batch) = store.get(&first_key) else {
            panic!("first batch should still exist");
        };
        let Some(record) = batch
            .records
            .iter()
            .find(|record| record.key == stable_member)
        else {
            panic!("stable member should remain in the first batch");
        };
        assert_eq!(record.instance.packed_path_index, u32::MAX);
    }

    #[test]
    fn atlas_rebuild_compacts_surviving_panel_line_paths() {
        let mut store = ShapeBatchStore::default();
        let key = test_batch_key();
        let first_panel = Entity::from_bits(1);
        let second_panel = Entity::from_bits(2);
        store.upsert_panel(
            first_panel,
            vec![(key.clone(), test_batch_record(first_panel, 0))],
        );
        store.upsert_panel(
            second_panel,
            vec![(key.clone(), test_batch_record(second_panel, 1))],
        );
        store.rebuild_path_atlas_if_dirty();
        let Some(batch) = store.get(&key) else {
            panic!("batch should exist");
        };
        let first_indices: Vec<u32> = batch
            .instances()
            .iter()
            .map(|record| record.packed_path_index)
            .collect();
        assert_eq!(first_indices, vec![0, 1]);

        store.remove_panel(first_panel);
        store.rebuild_path_atlas_if_dirty();

        let Some(batch) = store.get(&key) else {
            panic!("batch should still exist");
        };
        let second_indices: Vec<u32> = batch
            .instances()
            .iter()
            .map(|record| record.packed_path_index)
            .collect();
        assert_eq!(second_indices, vec![0]);
    }

    #[test]
    fn line_batch_bounds_use_instance_rects_and_run_transforms() {
        let mut batch = ShapeBatch::default();
        let mut record = test_batch_record(Entity::from_bits(1), 0);
        record.instance.rect_min = Vec2::new(1.0, 2.0);
        record.instance.rect_size = Vec2::new(3.0, 4.0);
        record.run.transform = Mat4::from_translation(Vec3::new(10.0, 20.0, 0.0));
        batch.push_record(record);

        let bounds = batch.world_bounds();

        assert_eq!(
            bounds,
            Some((Vec3::new(11.0, 22.0, 0.0), Vec3::new(14.0, 26.0, 0.0))),
        );
    }

    #[test]
    fn material_slot_refresh_dirties_only_line_run_records() {
        let mut batch = ShapeBatch::default();
        let original = test_batch_record(Entity::from_bits(1), 0);
        let mut refreshed = test_batch_record(Entity::from_bits(1), 0);
        let Ok(slot) = crate::render::material_table::MaterialSlotId::try_from(7) else {
            panic!("test slot is not the sentinel");
        };
        refreshed.run.material = slot.into();
        batch.push_record(original);
        batch.clear_render_record_dirty();
        batch.clear_path_quad_dirty();
        batch.clear_bounds_dirty();

        batch.refresh_record(refreshed);

        assert!(batch.render_records_are_dirty());
        assert!(!batch.path_quads_are_dirty());
        assert!(!batch.bounds_are_dirty());
    }

    #[test]
    fn line_record_uses_shape_local_box_uv_not_atlas_uv() {
        let mut app = line_batch_app();
        app.world_mut().spawn(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(60.0))
                .with_tree(one_line_tree(horizontal_line()))
                .build()
                .unwrap_or_else(|error| panic!("line panel should build: {error:?}")),
        );
        settle(&mut app);

        let store = app.world().resource::<ShapeBatchStore>();
        let Some((_, batch)) = store.batches().next() else {
            panic!("one line batch should exist");
        };
        let Some(record) = batch.records.first() else {
            panic!("line batch should have a record");
        };
        assert_ne!(record.instance.uv_min, Vec2::ZERO);
        assert_ne!(record.instance.uv_size, Vec2::ONE);
        assert_eq!(record.instance.box_uv_min, record.instance.uv_min);
        assert_eq!(record.instance.box_uv_size, record.instance.uv_size);
    }

    #[test]
    fn panel_shape_merge_key_keeps_material_source_without_scalar_color_values() {
        let red_color = Color::srgb(1.0, 0.0, 0.0);
        let blue_color = Color::srgb(0.0, 0.0, 1.0);
        let mut red = test_line(0);
        red.primitives[0].color = red_color;
        let mut red_again = test_line(0);
        red_again.primitives[0].color = red_color;
        let mut blue = test_line(0);
        blue.primitives[0].color = blue_color;
        let commands = vec![RenderCommand {
            bounds:      red.visual_bounds(),
            element_idx: 0,
            kind:        RenderCommandKind::PanelShapes {
                shapes: vec![red.clone()],
            },
            z_index:     DrawZIndex::default(),
        }];
        let draw_depth = draw_depth_for_command(&commands, 0);
        let material = Handle::<StandardMaterial>::default();
        let red_source = ShapePrimitiveSource {
            element_index: 0,
            draw_depth,
            line: &red,
            primitive: &red.primitives()[0],
        };
        let red_same_source = ShapePrimitiveSource {
            element_index: 0,
            draw_depth,
            line: &red_again,
            primitive: &red_again.primitives()[0],
        };
        let red_different_source = ShapePrimitiveSource {
            element_index: 0,
            draw_depth,
            line: &red_again,
            primitive: &red_again.primitives()[0],
        };
        let blue_same_source = ShapePrimitiveSource {
            element_index: 0,
            draw_depth,
            line: &blue,
            primitive: &blue.primitives()[0],
        };
        let blue_different_source = ShapePrimitiveSource {
            element_index: 0,
            draw_depth,
            line: &blue,
            primitive: &blue.primitives()[0],
        };

        assert_eq!(
            PanelShapeMergeKey::from_source(&red_source, &material, VisualShadow::Cast),
            PanelShapeMergeKey::from_source(&red_same_source, &material, VisualShadow::Cast),
        );
        assert_eq!(
            PanelShapeMergeKey::from_source(&red_source, &material, VisualShadow::Cast),
            PanelShapeMergeKey::from_source(&red_different_source, &material, VisualShadow::Cast),
        );
        assert_ne!(
            PanelShapeMergeKey::from_source(&red_source, &material, VisualShadow::Cast),
            PanelShapeMergeKey::from_source(&blue_same_source, &material, VisualShadow::Cast),
        );
        assert_ne!(
            PanelShapeMergeKey::from_source(&red_source, &material, VisualShadow::Cast),
            PanelShapeMergeKey::from_source(&blue_different_source, &material, VisualShadow::Cast),
        );
    }

    #[test]
    fn panel_material_input_projects_color_and_scalar_values() {
        let mut base = StandardMaterial {
            metallic: 0.42,
            perceptual_roughness: 0.23,
            reflectance: 0.61,
            ..Default::default()
        };
        render::apply_sidedness(&mut base, Sidedness::BothSides);
        let color = Color::srgb(0.2, 0.4, 0.6);
        let line = test_line(0);
        let input = PanelShapeMaterialSlotInput {
            key:           PanelShapeRenderKey {
                panel:  Entity::from_bits(10),
                source: line.primitives()[0].source_key(),
            },
            base_material: &base,
            fill_color:    color,
            alpha_mode:    AlphaMode::Blend,
            lighting:      Lighting::Lit,
            sidedness:     Sidedness::BothSides,
        };
        let mut expected_material = base.clone();
        expected_material.base_color = color;
        expected_material.alpha_mode = AlphaMode::Blend;
        render::apply_sidedness(&mut expected_material, Sidedness::BothSides);
        let expected_values =
            crate::render::material_table::MaterialSlotValues::from(&expected_material);

        let candidate = input.material_slot_candidate();

        assert_eq!(candidate.values, expected_values);
    }

    #[test]
    fn material_slot_input_uses_cascade_lighting_and_sidedness() {
        let mut base = StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..Default::default()
        };
        render::apply_sidedness(&mut base, Sidedness::BackOnly);
        let color = Color::srgb(0.2, 0.4, 0.6);
        let line = test_line(0);
        let input = PanelShapeMaterialSlotInput {
            key:           PanelShapeRenderKey {
                panel:  Entity::from_bits(10),
                source: line.primitives()[0].source_key(),
            },
            base_material: &base,
            fill_color:    color,
            alpha_mode:    base.alpha_mode,
            lighting:      Lighting::Lit,
            sidedness:     Sidedness::BothSides,
        };
        let mut expected_material = base.clone();
        expected_material.base_color = color;
        expected_material.unlit = false;
        render::apply_sidedness(&mut expected_material, Sidedness::BothSides);
        let source_candidate = MaterialSlotCandidate::from(&base);
        let expected_candidate = MaterialSlotCandidate::from(&expected_material);

        let candidate = input.material_slot_candidate();

        assert_eq!(candidate.values, expected_candidate.values);
        assert_eq!(
            candidate.pipeline_compatibility,
            expected_candidate.pipeline_compatibility,
        );
        assert_ne!(
            candidate.pipeline_compatibility,
            source_candidate.pipeline_compatibility,
        );
    }

    fn test_batch_key() -> PathBatchKey {
        let mut material = StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            ..Default::default()
        };
        render::apply_sidedness(&mut material, Sidedness::BothSides);
        PathBatchKey {
            z_index:                DrawZIndex::default(),
            z_index_rank:           DrawZIndexRank::default(),
            batch_family:           DrawBatchFamily::PanelShape,
            shadow:                 VisualShadow::Cast,
            layers:                 BatchRenderLayers(RenderLayers::layer(0)),
            pipeline_compatibility: batch_key::PipelineCompatibility::from(&material),
            resource_compatibility: batch_key::ResourceCompatibility::from(&material),
        }
    }

    fn test_batch_record(panel: Entity, primitive_ordinal: usize) -> ShapeBatchRecord {
        ShapeBatchRecord {
            key:              PanelShapeRenderKey {
                panel,
                source: PanelShapePrimitiveKey::new(
                    PanelShapeSourceKey::element(primitive_ordinal, 0, 0),
                    0,
                ),
            },
            draw_order_index: DrawOrderIndex::from(primitive_ordinal),
            outline:          test_outline(),
            instance:         PathQuadRecord {
                rect_min:          Vec2::ZERO,
                rect_size:         Vec2::ONE,
                uv_min:            Vec2::ZERO,
                uv_size:           Vec2::ONE,
                box_uv_min:        Vec2::ZERO,
                box_uv_size:       Vec2::ONE,
                packed_path_index: 0,
                render_index:      0,
                box_uv_flip_x:     0,
            },
            run:              PathRenderRecord {
                transform:          Mat4::IDENTITY,
                material:           SdfPaintMaterial::NotAuthored.to_gpu(),
                render_mode:        u32::from(RenderMode::Text),
                clip_depth_nudge:   0.0,
                oit_depth_offset:   0.0,
                aa_flags:           AntiAlias::Both.aa_flags(),
                text_coverage_bias: 0.0,
                text_em_height:     0.0,
            },
        }
    }

    fn test_outline() -> PathOutline {
        PathOutline {
            bounds:   Bounds {
                min: Vec2::ZERO,
                max: Vec2::ONE,
            },
            contours: vec![PathContour {
                min_feature:   0.0,
                fade_exponent: 0.0,
                segments:      vec![
                    QuadraticSegment {
                        start:   Vec2::ZERO,
                        control: Vec2::new(0.5, 0.0),
                        end:     Vec2::X,
                    },
                    QuadraticSegment {
                        start:   Vec2::X,
                        control: Vec2::new(1.0, 0.5),
                        end:     Vec2::ONE,
                    },
                    QuadraticSegment {
                        start:   Vec2::ONE,
                        control: Vec2::new(0.5, 1.0),
                        end:     Vec2::Y,
                    },
                    QuadraticSegment {
                        start:   Vec2::Y,
                        control: Vec2::new(0.0, 0.5),
                        end:     Vec2::ZERO,
                    },
                ],
            }],
        }
    }

    fn test_line(line_ordinal: usize) -> ResolvedPanelShape {
        let source_key = PanelShapeSourceKey::element(0, 0, line_ordinal);
        let primitive = ResolvedPanelShapePrimitive {
            source_key: PanelShapePrimitiveKey::new(source_key, 0),
            kind:       PanelShapePrimitiveKind::Segment,
            geometry:   PanelShapePrimitiveGeometry::Segment {
                start: Vec2::ZERO,
                end:   Vec2::X,
                width: 1.0,
            },
            color:      Color::WHITE,
            material:   Cascade::Inherit,
            bounds:     BoundingBox {
                x:      0.0,
                y:      0.0,
                width:  1.0,
                height: 1.0,
            },
            clip:       None,
            part_order: 0,
        };
        ResolvedPanelShape {
            source_key,
            source_command_index: 0,
            owner_bounds: BoundingBox {
                x:      0.0,
                y:      0.0,
                width:  1.0,
                height: 1.0,
            },
            visual_bounds: primitive.bounds(),
            clip: None,
            start: Vec2::ZERO,
            end: Vec2::X,
            shaft_start: Vec2::ZERO,
            shaft_end: Vec2::X,
            width: 1.0,
            color: Color::WHITE,
            material: Cascade::Inherit,
            hairline_fade: Cascade::Inherit,
            shadow_casting: Cascade::Inherit,
            primitives: vec![primitive],
        }
    }

    fn draw_depth_for_command(
        commands: &[RenderCommand],
        command_index: usize,
    ) -> DrawCommandDepth {
        let draw_order = DrawOrder::from_commands(commands);
        match draw_order.depth_for(command_index) {
            Some(draw_depth) => draw_depth,
            None => panic!("line command should receive draw depth"),
        }
    }

    // ── Widget visual-slot overrides ────────────────────────────────────────

    fn widget_line_batch_app() -> App {
        let mut app = line_batch_app();
        app.add_plugins(WidgetsPlugin);
        app
    }

    fn spawn_slotted_widget_line_panel(app: &mut App, transform: Transform) -> Entity {
        let tree = LayoutBuilder::with_widget_root(
            El::new()
                .size(40.0, 20.0)
                .draw(PanelDraw::lines([horizontal_line()]))
                .button("styled")
                .visual_slot(SHAPE_SLOT),
        )
        .build();
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(60.0))
            .with_tree(tree)
            .build()
            .unwrap_or_else(|error| panic!("slotted line panel should build: {error:?}"));
        app.world_mut().spawn((panel, transform)).id()
    }

    fn spawn_stateful_text_and_draw_panel(app: &mut App) -> Entity {
        let mut builder = LayoutBuilder::with_widget_root(
            El::new()
                .size(40.0, 20.0)
                .draw(PanelDraw::lines([horizontal_line()]))
                .button("styled")
                .hovered(Appearance::new().path_color(SHAPE_OVERRIDE_COLOR)),
        );
        builder.text(Text::new("Alpha", TextStyle::new(10.0)));
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(60.0))
            .with_tree(builder.build())
            .build()
            .unwrap_or_else(|error| panic!("text and draw panel should build: {error:?}"));
        app.world_mut().spawn((panel, Transform::default())).id()
    }

    fn styled_widget(app: &mut App) -> Entity {
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<PanelWidget>>();
        let widgets: Vec<Entity> = query.iter(app.world()).collect();
        assert_eq!(widgets.len(), 1, "expected exactly one reified widget");
        widgets[0]
    }

    fn set_shape_slot_override(app: &mut App, widget: Entity, value: VisualSlotOverride) {
        let mut overrides = WidgetVisualOverrides::default();
        overrides.set(SHAPE_SLOT, value);
        app.world_mut().entity_mut(widget).insert(overrides);
    }

    fn one_shape_batch_state(app: &App) -> (BatchAlphaMode, Option<Entity>) {
        let store = app.world().resource::<ShapeBatchStore>();
        let batches: Vec<(&PathBatchKey, &ShapeBatch)> = store.batches().collect();
        assert_eq!(batches.len(), 1, "expected exactly one panel-line batch");
        let (key, batch) = batches[0];
        (key.pipeline_compatibility.alpha, batch.entity)
    }

    #[test]
    fn shape_path_color_override_patches_material_row_without_rekeying() {
        let mut app = widget_line_batch_app();
        spawn_slotted_widget_line_panel(&mut app, Transform::default());
        settle(&mut app);
        let (_, entity_before) = one_shape_batch_state(&app);
        assert_eq!(
            first_shape_material_values(&app).base_color,
            material_base_color(Color::WHITE),
        );

        let widget = styled_widget(&mut app);
        set_shape_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_path_color(SHAPE_OVERRIDE_COLOR),
        );
        app.update();

        assert_eq!(
            first_shape_material_values(&app).base_color,
            material_base_color(SHAPE_OVERRIDE_COLOR),
        );
        let (_, entity_after) = one_shape_batch_state(&app);
        assert_eq!(
            entity_after, entity_before,
            "a color change must not re-key"
        );

        app.world_mut()
            .get_mut::<WidgetVisualOverrides>(widget)
            .unwrap_or_else(|| panic!("widget should keep its override component"))
            .clear(SHAPE_SLOT);
        app.update();
        assert_eq!(
            first_shape_material_values(&app).base_color,
            material_base_color(Color::WHITE),
        );
    }

    #[test]
    fn text_color_override_leaves_shape_material_rows_authored() {
        let mut app = widget_line_batch_app();
        spawn_slotted_widget_line_panel(&mut app, Transform::default());
        settle(&mut app);
        let widget = styled_widget(&mut app);

        set_shape_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_text_color(SHAPE_OVERRIDE_COLOR),
        );
        app.update();

        assert_eq!(
            first_shape_material_values(&app).base_color,
            material_base_color(Color::WHITE),
        );
    }

    #[test]
    fn state_path_color_updates_the_draw_row_without_a_text_override() {
        let mut app = widget_line_batch_app();
        let panel = spawn_stateful_text_and_draw_panel(&mut app);
        settle(&mut app);
        let widget = styled_widget(&mut app);
        let Some(slots) = app.world().get::<WidgetVisualSlots>(widget) else {
            panic!("widget should carry visual slots");
        };
        let Some((element_index, _)) = slots
            .elements()
            .iter()
            .find(|(_, capabilities)| capabilities.contains(VisualElementCapabilities::DRAW))
        else {
            panic!("draw recipient should remain in visual slots");
        };
        let element_index = *element_index;

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(
            first_shape_material_values(&app).base_color,
            material_base_color(SHAPE_OVERRIDE_COLOR),
        );
        let Some(override_value) = app
            .world()
            .resource::<VisualOverrideIndex>()
            .get(panel, element_index)
        else {
            panic!("hovered draw recipient should receive an override");
        };
        assert_eq!(override_value.path_color, Some(SHAPE_OVERRIDE_COLOR));
        assert_eq!(override_value.text_color, None);
    }

    #[test]
    fn shape_material_override_rekeys_group_and_retires_the_emptied_batch() {
        let mut app = widget_line_batch_app();
        let opaque_material = material_asset(
            &mut app,
            StandardMaterial {
                alpha_mode: AlphaMode::Opaque,
                ..Default::default()
            },
        );
        spawn_slotted_widget_line_panel(&mut app, Transform::default());
        settle(&mut app);
        let (alpha_before, entity_before) = one_shape_batch_state(&app);
        let origin_entity =
            entity_before.unwrap_or_else(|| panic!("settled panel-line batch should be spawned"));
        assert_ne!(alpha_before, BatchAlphaMode::Opaque);

        let widget = styled_widget(&mut app);
        set_shape_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_material(opaque_material),
        );
        app.update();

        let (alpha_after, entity_after) = one_shape_batch_state(&app);
        assert_eq!(alpha_after, BatchAlphaMode::Opaque);
        let destination_entity = entity_after
            .unwrap_or_else(|| panic!("destination panel-line batch should be spawned"));
        assert_ne!(destination_entity, origin_entity);
        assert!(
            app.world().get_entity(origin_entity).is_err(),
            "the emptied origin batch should be retired",
        );
        let pickable = app
            .world()
            .get::<Pickable>(destination_entity)
            .unwrap_or_else(|| panic!("destination batch entity should carry Pickable::IGNORE"));
        assert!(!pickable.is_hoverable);
        assert!(!pickable.should_block_lower);

        app.world_mut()
            .get_mut::<WidgetVisualOverrides>(widget)
            .unwrap_or_else(|| panic!("widget should keep its override component"))
            .clear(SHAPE_SLOT);
        app.update();
        let (alpha_restored, _) = one_shape_batch_state(&app);
        assert_eq!(alpha_restored, alpha_before);
        assert!(
            app.world().get_entity(destination_entity).is_err(),
            "the emptied destination batch should be retired",
        );
    }

    fn single_shape_record(app: &App) -> (PathRenderRecord, PathQuadRecord) {
        let store = app.world().resource::<ShapeBatchStore>();
        let records: Vec<&ShapeBatchRecord> = store
            .batches()
            .flat_map(|(_, batch)| batch.records.iter())
            .collect();
        assert_eq!(records.len(), 1, "expected exactly one panel-line record");
        (records[0].run, records[0].instance)
    }

    fn shape_upload_count(app: &App) -> usize {
        app.world()
            .resource::<DiegeticPerfStats>()
            .line_batch
            .uploads
            .get()
    }

    #[test]
    fn shape_offset_override_moves_the_record_in_panel_space() {
        let mut app = widget_line_batch_app();
        let panel = spawn_slotted_widget_line_panel(
            &mut app,
            Transform::from_rotation(Quat::from_rotation_z(FRAC_PI_2)),
        );
        settle(&mut app);
        assert_eq!(
            shape_upload_count(&app),
            0,
            "settled pipeline should be quiet"
        );
        let (record_before, instance_before) = single_shape_record(&app);
        let widget = styled_widget(&mut app);
        let widget_transform = *app
            .world()
            .get::<Transform>(widget)
            .unwrap_or_else(|| panic!("widget should carry a transform"));
        let panel_tick = app
            .world()
            .entity(panel)
            .get_ref::<DiegeticPanel>()
            .unwrap_or_else(|| panic!("panel role should be live"))
            .last_changed();
        let computed_tick = app
            .world()
            .entity(panel)
            .get_ref::<ComputedDiegeticPanel>()
            .unwrap_or_else(|| panic!("panel should have computed output"))
            .last_changed();

        set_shape_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_offset(SHAPE_SLOT_OFFSET),
        );
        app.update();

        // The offset is panel-local: it composes to the right of the rotated
        // panel's global matrix, not as a world-axis translation.
        let panel_matrix = app
            .world()
            .get::<GlobalTransform>(panel)
            .unwrap_or_else(|| panic!("panel should carry a global transform"))
            .to_matrix();
        let (record_after, instance_after) = single_shape_record(&app);
        assert_eq!(
            record_after.transform,
            panel_matrix * Mat4::from_translation(SHAPE_SLOT_OFFSET.extend(0.0)),
        );
        assert_eq!(
            instance_after, instance_before,
            "authored quad geometry stays untouched",
        );
        assert_eq!(
            app.world().get::<Transform>(widget),
            Some(&widget_transform),
            "an offset override must not move the widget transform",
        );
        assert_eq!(
            app.world()
                .entity(panel)
                .get_ref::<DiegeticPanel>()
                .unwrap_or_else(|| panic!("panel role should be live"))
                .last_changed(),
            panel_tick,
            "an override must not fire Changed<DiegeticPanel>",
        );
        assert_eq!(
            app.world()
                .entity(panel)
                .get_ref::<ComputedDiegeticPanel>()
                .unwrap_or_else(|| panic!("panel should have computed output"))
                .last_changed(),
            computed_tick,
            "an override must not fire Changed<ComputedDiegeticPanel>",
        );

        // Author the identical override a second time: the reconcile rebuilds
        // an equal record, so nothing re-uploads and nothing changes.
        set_shape_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_offset(SHAPE_SLOT_OFFSET),
        );
        app.update();
        assert_eq!(
            shape_upload_count(&app),
            0,
            "a repeated identical override is a no-op",
        );
        assert_eq!(single_shape_record(&app), (record_after, instance_after));

        app.world_mut()
            .get_mut::<WidgetVisualOverrides>(widget)
            .unwrap_or_else(|| panic!("widget should keep its override component"))
            .clear(SHAPE_SLOT);
        app.update();
        let (record_restored, instance_restored) = single_shape_record(&app);
        assert_eq!(
            record_restored.transform, record_before.transform,
            "clearing the override restores the authored record transform",
        );
        assert_eq!(instance_restored, instance_before);
    }
}
