//! Batched SDF fill route backed by the frame material table.
//!
//! Builds typed SDF fill/border records and their material-table rows in one
//! frame-local pass. Production panel backgrounds, borders, and divider
//! rectangles render through visible `DiegeticSdfFillBatch` entities.

use std::collections::HashMap;
use std::collections::HashSet;

use bevy::asset::Asset;
use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::visibility::VisibilitySystems;
use bevy::light::NotShadowCaster;
use bevy::mesh::Indices;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::ExtendedMaterial;
use bevy::pbr::MATERIAL_BIND_GROUP_INDEX;
use bevy::pbr::MaterialExtension;
use bevy::pbr::MaterialExtensionKey;
use bevy::pbr::MaterialExtensionPipeline;
use bevy::pbr::MaterialPlugin;
use bevy::pbr::MeshMaterial3d;
use bevy::pbr::StandardMaterial;
use bevy::picking::Pickable;
use bevy::prelude::AlphaMode;
use bevy::prelude::App;
use bevy::prelude::AssetServer;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Handle;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Mat4;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectResource;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Transform;
use bevy::prelude::UVec4;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Vec4;
use bevy::prelude::Vec4Swizzles;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy::reflect::TypePath;
use bevy::render::render_resource::AsBindGroup;
use bevy::render::render_resource::PrimitiveTopology;
use bevy::render::render_resource::RenderPipelineDescriptor;
use bevy::render::render_resource::ShaderSize;
use bevy::render::render_resource::ShaderType;
use bevy::render::render_resource::SpecializedMeshPipelineError;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use bevy::transform::TransformSystems;
use hana_kana::ToU32;
use hana_kana::ToUsize;

use super::BatchAlphaMode;
use super::BatchRenderLayers;
use super::CommandIndex;
use super::Dirty;
use super::ElementIndex;
use super::batch_key;
use super::batch_key::PipelineCompatibility;
use super::batch_key::ResourceCompatibility;
use super::batch_key::VisualShadow;
use super::batch_store;
use super::batch_store::Batch;
use super::batch_store::BatchEntry;
use super::batch_store::BatchStore;
use super::batch_store::MemberBatch;
use super::batch_store::MemberFamily;
use super::batch_store::MemberRecord;
#[cfg(test)]
use super::draw_order;
use super::draw_order::DrawCommandDepth;
use super::draw_order::DrawOrderIndex;
use super::draw_order::DrawZIndexRank;
use super::material;
use super::material_table;
use super::material_table::BatchAssetWrites;
use super::material_table::BatchResourcesReady;
use super::material_table::FrameMaterialTableBuilder;
use super::material_table::GpuMaterialSlotId;
use super::material_table::MaterialSlotAppend;
use super::material_table::MaterialSlotCandidate;
use super::material_table::MaterialSlotId;
use super::material_table::MaterialSlotInput;
use super::material_table::MaterialTableAppendReady;
use super::material_table::RetainedBatchBufferUploads;
#[cfg(test)]
use super::material_table::SdfDriverRunOrder;
use super::material_table::SdfPaintMaterial;
use super::panel_geometry::ResolvedSdfSurface;
use super::panel_geometry::ResolvedSdfSurfaceRegistry;
use super::panel_geometry::SdfRecordRole;
use super::panel_geometry::SurfaceEvictionCause;
use crate::DrawZIndex;
use crate::cascade::Resolved;
use crate::cascade::SdfMaterial;
use crate::constants::EMBEDDED_SDF_PANEL_BATCH_SHADER_PATH;
use crate::layout::DrawBatchFamily;
use crate::layout::Lighting;
use crate::layout::ShadowCasting;
use crate::layout::Sidedness;
use crate::panel::DiegeticPanel;
use crate::panel::DiegeticPerfStats;
use crate::panel::DroppedSdfSurfaces;
use crate::render;
use crate::widgets::VisualOverrideIndex;
use crate::widgets::VisualSlotOverride;

/// Extra `clip_depth_nudge` layer-units pushing an opaque/alpha-mask SDF fill away
/// from the camera so coplanar panel text, drawn at its true depth, wins the
/// depth test instead of z-fighting the fill. Only the depth-buffer regime
/// (`Opaque`/`Mask`) applies it; transparent and OIT fills order through
/// `ScreenDepthBias`/`OitDepthOffset` and get zero here. Starts far below one
/// layer-unit; raise by powers of ten until text shows without the fill
/// visibly floating off the surface.
const OPAQUE_FILL_DEPTH_PUSH_LAYERS: f32 = 1.0;

/// Render material used by batched SDF fill and border records.
pub(crate) type SdfExtendedMaterial = ExtendedMaterial<StandardMaterial, SdfExtension>;

/// Marker inserted on private SDF fill batch entities, BRP-inspectable.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub(super) struct DiegeticSdfFillBatch;

/// BRP-inspectable snapshot of one built GPU SDF record's resolved fields.
#[derive(Clone, Copy, Debug, Reflect)]
pub(super) struct SdfRecordSnapshot {
    /// `SdfRenderRecord::fill_material` slot id (`u32::MAX` when not authored).
    pub fill_material:   u32,
    /// `SdfRenderRecord::border_material` slot id (`u32::MAX` when not authored).
    pub border_material: u32,
    /// `SdfRenderRecord::paint_mask` role bits.
    pub paint_mask:      u32,
    /// SDF rounded-rect half-size.
    pub half_size:       Vec2,
    /// Quad half-size including AA padding.
    pub mesh_half_size:  Vec2,
    /// SDF fragment flags.
    pub flags:           u32,
}

/// BRP-inspectable summary of one live `SdfBatchKey` entry.
#[derive(Debug, Default, Reflect)]
pub(super) struct SdfBatchSummary {
    /// `SdfBatchKey::z_index` widened for BRP.
    pub z_index:                DrawZIndex,
    /// Active `RenderLayers` indices copied from `SdfBatchKey::layers`.
    pub render_layers:          Vec<u32>,
    /// Debug label for `SdfBatchKey::shadow`.
    pub shadow:                 String,
    /// `ContiguousDrawnRun::value` for this batch.
    pub contiguous_run:         u32,
    /// Live `SdfBatch::records` length.
    pub record_count:           u32,
    /// Debug label for `SdfBatchKey::pipeline_compatibility`.
    pub pipeline_compatibility: String,
    /// Debug label for `SdfBatchKey::resource_compatibility`.
    pub resource_compatibility: String,
}

/// Per-frame diagnostic snapshot of built SDF GPU records, BRP-inspectable.
#[derive(Debug, Default, Reflect, Resource)]
#[reflect(Resource)]
pub(super) struct SdfRecordDiagnostics {
    /// Per-batch decomposition of the live SDF batch set.
    pub batches: Vec<SdfBatchSummary>,
    /// Live (non-padding) record snapshots committed this frame.
    pub records: Vec<SdfRecordSnapshot>,
}

/// One SDF role inside a resolved panel surface.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum SdfMaterialRole {
    /// Panel background or element fill.
    Fill,
    /// Panel or element border ring.
    Border,
}

/// Material source identity for one SDF fill or border role.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct SdfMaterialSourceKey {
    /// Panel entity that owns this role.
    pub panel:           Entity,
    /// Source element in the panel's layout tree.
    pub element:         ElementIndex,
    /// Adjacent-child pair ordinal within `element`; `None` means this role belongs to the
    /// element itself.
    pub divider_ordinal: Option<usize>,
    /// Role within the resolved SDF surface.
    pub role:            SdfMaterialRole,
}

/// Append-time source material for one SDF material-table role.
struct SdfMaterialSlotInput<'a> {
    /// Source identity returned with the retained slot.
    key:            SdfMaterialSourceKey,
    /// Resolved `StandardMaterial` source; defaults are folded before this input exists.
    base_material:  &'a StandardMaterial,
    /// Layout color override for `StandardMaterial::base_color`.
    color_override: Option<Color>,
    /// Panel cascade-resolved lighting policy applied over the source material.
    lighting:       Lighting,
    /// Panel cascade-resolved sidedness applied over the source material.
    sidedness:      Sidedness,
}

impl MaterialSlotInput for SdfMaterialSlotInput<'_> {
    type Key = SdfMaterialSourceKey;

    fn key(&self) -> Self::Key { self.key }

    fn material_slot_candidate(&self) -> MaterialSlotCandidate {
        let mut material = self.base_material.clone();
        if let Some(color) = self.color_override {
            material.base_color = color;
        }
        render::apply_sidedness(&mut material, self.sidedness);
        material.unlit = matches!(self.lighting, Lighting::Unlit);
        MaterialSlotCandidate {
            values:                 (&material).into(),
            pipeline_compatibility: (&material).into(),
            resource_compatibility: ResourceCompatibility::from(&material),
        }
    }
}

/// Identity for one retained SDF surface record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct SdfRecordKey {
    /// Panel entity that owns this record.
    pub panel:           Entity,
    /// Source element in the panel's layout tree.
    pub element:         ElementIndex,
    /// Adjacent-child pair ordinal for a child divider; `None` for an element surface.
    pub divider_ordinal: Option<usize>,
    /// Role produced when clipping separates a fill from its border.
    pub role:            SdfRecordRole,
}

/// Panel-local rounded-rectangle SDF half-size in world units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SdfHalfSize {
    /// Half-size value written to `SdfRenderRecord::half_size`.
    pub value: Vec2,
}

/// Panel-local SDF mesh half-size, including `SDF_AA_PADDING`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MeshHalfSize {
    /// Half-size value written to `SdfRenderRecord::mesh_half_size`.
    pub value: Vec2,
}

/// Panel-local SDF clip rectangle after AA padding expansion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LocalClipRect {
    /// `[left, bottom, right, top]` clip rectangle.
    pub value: Vec4,
}

/// Panel-local per-corner SDF radii.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CornerRadii {
    /// `[top_left, top_right, bottom_right, bottom_left]` radii.
    pub value: Vec4,
}

/// Panel-local per-side SDF border widths.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BorderWidths {
    /// `[top, right, bottom, left]` widths.
    pub value: Vec4,
}

/// Fill and border role-presence bits read by `sdf_panel.wgsl`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, ShaderType)]
pub(super) struct SdfPaintMask {
    /// Bit field written to `SdfRenderRecord::paint_mask`.
    bits: u32,
}

impl SdfPaintMask {
    /// Fill role bit in `SdfPaintMask`.
    const FILL: u32 = 1;
    /// Border role bit in `SdfPaintMask`.
    const BORDER: u32 = 1 << 1;

    /// Builds role bits from CPU-side SDF material presence.
    #[must_use]
    const fn from_materials(fill: SdfPaintMaterial, border: SdfPaintMaterial) -> Self {
        let fill_bit = match fill {
            SdfPaintMaterial::Authored(_) => Self::FILL,
            SdfPaintMaterial::NotAuthored => 0,
        };
        let border_bit = match border {
            SdfPaintMaterial::Authored(_) => Self::BORDER,
            SdfPaintMaterial::NotAuthored => 0,
        };
        Self {
            bits: fill_bit | border_bit,
        }
    }

    /// Returns the raw bit field written to WGSL.
    #[must_use]
    const fn bits(self) -> u32 { self.bits }

    /// Returns an empty role mask for capacity-tail records.
    #[must_use]
    const fn empty() -> Self { Self { bits: 0 } }
}

/// Per-command monotonic run ordinal used to preserve cross-batch order.
///
/// Example: panel order is `background`, `text`, `border`; `background` and
/// `border` are both SDF-compatible, but they must not merge into one SDF
/// batch because that would draw `background`, `border`, `text`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(super) struct ContiguousDrawnRun {
    /// Frame-local run ordinal.
    pub value: u32,
}

/// Key for one SDF fill batch and one `SdfBatchResources` entry.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct SdfBatchKey {
    /// Authored z-index splitter for this batch's SDF records.
    pub z_index:                DrawZIndex,
    /// Dense panel-local rank for `z_index`, used by the batch material
    /// `StandardMaterial::depth_bias`.
    pub z_index_rank:           DrawZIndexRank,
    /// Renderer family that owns this SDF batch.
    pub batch_family:           DrawBatchFamily,
    /// Render layers copied from the panel.
    pub layers:                 BatchRenderLayers,
    /// Shadow participation for this batch.
    pub shadow:                 VisualShadow,
    /// Maximal compatible command run in `DrawCommandDepth::draw_order_index()` order.
    pub contiguous_drawn_run:   ContiguousDrawnRun,
    /// Material-derived pipeline facts that must agree inside this SDF draw.
    pub pipeline_compatibility: PipelineCompatibility,
    /// Texture and bind-group facts copied into the SDF render material.
    pub resource_compatibility: ResourceCompatibility,
}

/// Material slots and compatibility values appended for one SDF record.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SdfRecordMaterialSlots {
    /// Fill role material row or `SdfPaintMaterial::NotAuthored`.
    pub fill:                   SdfPaintMaterial,
    /// Border role material row or `SdfPaintMaterial::NotAuthored`.
    pub border:                 SdfPaintMaterial,
    /// Pipeline compatibility selected for the enclosing SDF batch.
    pub pipeline_compatibility: PipelineCompatibility,
    /// Resource compatibility selected for the enclosing SDF batch.
    pub resource_compatibility: ResourceCompatibility,
}

/// CPU-side SDF record retained by `SdfBatchStore`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ResolvedSdfBatchRecord {
    /// Retained-record identity.
    pub record_key:       SdfRecordKey,
    /// Original command order used to break equal-depth draw ties.
    pub draw_order:       CommandIndex,
    /// Fill material source identity.
    pub fill_source:      SdfMaterialSourceKey,
    /// Border material source identity.
    pub border_source:    SdfMaterialSourceKey,
    /// `DrawCommandDepth` for this command.
    pub draw_depth:       DrawCommandDepth,
    /// Batch key selected while appending frame material rows.
    pub batch_key:        SdfBatchKey,
    /// Panel-local transform from `ResolvedSdfSurface`.
    pub local_transform:  Transform,
    /// World transform written after `TransformSystems::Propagate`.
    pub transform:        Mat4,
    /// Rounded-rectangle SDF half-size.
    pub half_size:        SdfHalfSize,
    /// Mesh half-size including AA padding.
    pub mesh_half_size:   MeshHalfSize,
    /// Per-corner rounded-rectangle radii.
    pub corner_radii:     CornerRadii,
    /// Per-side border widths.
    pub border_widths:    BorderWidths,
    /// Local clip rectangle.
    pub clip_rect:        LocalClipRect,
    /// Fill material table row state.
    pub fill_material:    SdfPaintMaterial,
    /// Border material table row state.
    pub border_material:  SdfPaintMaterial,
    /// Role-present bits derived from `fill_material` and `border_material`.
    pub paint_mask:       SdfPaintMask,
    /// Clip-space depth nudge in layer units for non-OIT views.
    pub clip_depth_nudge: f32,
    /// Per-record OIT position-z offset for coplanar ordering.
    pub oit_depth_offset: f32,
    /// Extra shader flags for the SDF fragment path.
    pub flags:            u32,
}

impl ResolvedSdfBatchRecord {
    /// Builds a CPU-retained SDF record from one resolved panel-local surface.
    #[must_use]
    fn from_resolved(
        surface: &ResolvedSdfSurface<'_>,
        materials: SdfRecordMaterialSlots,
        contiguous_drawn_run: ContiguousDrawnRun,
    ) -> Self {
        let pipeline_compatibility = sdf_record_pipeline_compatibility(surface, &materials);
        let fill_source = SdfMaterialSourceKey {
            panel:           surface.panel_entity,
            element:         surface.element_index,
            divider_ordinal: surface.divider_ordinal,
            role:            SdfMaterialRole::Fill,
        };
        let border_source = SdfMaterialSourceKey {
            panel:           surface.panel_entity,
            element:         surface.element_index,
            divider_ordinal: surface.divider_ordinal,
            role:            SdfMaterialRole::Border,
        };
        let batch_key = SdfBatchKey {
            z_index: surface.draw_depth.z_index(),
            z_index_rank: surface.draw_depth.z_index_rank(),
            batch_family: DrawBatchFamily::SdfSurface,
            layers: BatchRenderLayers(surface.render_layers.clone()),
            shadow: surface.shadow_casting.into(),
            contiguous_drawn_run,
            pipeline_compatibility,
            resource_compatibility: materials.resource_compatibility.clone(),
        };
        let paint_mask = SdfPaintMask::from_materials(materials.fill, materials.border);
        Self {
            record_key: SdfRecordKey {
                panel:           surface.panel_entity,
                element:         surface.element_index,
                divider_ordinal: surface.divider_ordinal,
                role:            surface.record_role,
            },
            draw_order: surface.command_index,
            fill_source,
            border_source,
            draw_depth: surface.draw_depth,
            batch_key,
            local_transform: surface.local_transform,
            transform: surface.local_transform.to_matrix(),
            half_size: SdfHalfSize {
                value: surface.sdf_half_size,
            },
            mesh_half_size: MeshHalfSize {
                value: surface.mesh_half_size,
            },
            corner_radii: CornerRadii {
                value: Vec4::from_array(surface.corner_radii),
            },
            border_widths: BorderWidths {
                value: Vec4::from_array(surface.border_widths),
            },
            clip_rect: LocalClipRect {
                value: surface.clip_rect,
            },
            fill_material: materials.fill,
            border_material: materials.border,
            paint_mask,
            clip_depth_nudge: surface.draw_depth.clip_depth_nudge().get()
                - opaque_fill_depth_push(
                    pipeline_compatibility.alpha,
                    surface.shadow_casting.into(),
                ),
            oit_depth_offset: surface.draw_depth.oit_depth_offset().get(),
            flags: 0,
        }
    }

    /// Rewrites `SdfRenderRecord::transform` from the panel's propagated transform.
    fn update_world_transform(&mut self, panel_transform: &GlobalTransform) {
        self.transform = panel_transform.to_matrix() * self.local_transform.to_matrix();
    }

    /// Keeps this record allocated while preventing the SDF shader from drawing it.
    const fn mark_hidden(&mut self) { self.paint_mask = SdfPaintMask::empty(); }
}

impl MemberRecord for ResolvedSdfBatchRecord {
    fn panel(&self) -> Entity { self.record_key.panel }

    fn transform(&self) -> Mat4 { self.transform }

    fn update_world_transform(&mut self, panel_transform: &GlobalTransform) {
        Self::update_world_transform(self, panel_transform);
    }
}

fn sdf_record_pipeline_compatibility(
    surface: &ResolvedSdfSurface<'_>,
    materials: &SdfRecordMaterialSlots,
) -> PipelineCompatibility {
    let mut pipeline_compatibility = materials.pipeline_compatibility;
    if clipped_border_uses_transparent_phase(surface, materials)
        && opaque_fill_depth_push(pipeline_compatibility.alpha, surface.shadow_casting.into()) > 0.0
    {
        pipeline_compatibility.alpha = BatchAlphaMode::Blend;
    }
    pipeline_compatibility
}

fn clipped_border_uses_transparent_phase(
    surface: &ResolvedSdfSurface<'_>,
    materials: &SdfRecordMaterialSlots,
) -> bool {
    matches!(materials.fill, SdfPaintMaterial::NotAuthored)
        && matches!(materials.border, SdfPaintMaterial::Authored(_))
        && surface.clip_rect_limits_mesh()
}

/// GPU mirror for one batched SDF fill/border surface.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
struct SdfRenderRecord {
    /// World-space transform used by the vertex shader.
    transform:        Mat4,
    /// Rounded-rectangle SDF half-size in local record space.
    half_size:        Vec2,
    /// Quad half-size including AA padding in local record space.
    mesh_half_size:   Vec2,
    /// Per-corner radii `[top_left, top_right, bottom_right, bottom_left]`.
    corner_radii:     Vec4,
    /// Per-side widths `[top, right, bottom, left]`.
    border_widths:    Vec4,
    /// Local clip rectangle `[left, bottom, right, top]`.
    clip_rect:        Vec4,
    /// Fill material row, or `INVALID_GPU_MATERIAL_SLOT`.
    fill_material:    GpuMaterialSlotId,
    /// Border material row, or `INVALID_GPU_MATERIAL_SLOT`.
    border_material:  GpuMaterialSlotId,
    /// Role-present bits checked before material-table reads.
    paint_mask:       SdfPaintMask,
    /// Extra shader flags for SDF rendering.
    flags:            u32,
    /// Clip-space depth nudge in layer units for non-OIT views.
    clip_depth_nudge: f32,
    /// Per-record OIT position-z offset for coplanar ordering.
    oit_depth_offset: f32,
}

impl SdfRenderRecord {
    /// Converts the CPU retained record to the GPU storage-buffer mirror.
    #[must_use]
    fn from_resolved(
        record: &ResolvedSdfBatchRecord,
        first_draw_order_index: DrawOrderIndex,
    ) -> Self {
        Self {
            transform:        record.transform,
            half_size:        record.half_size.value,
            mesh_half_size:   record.mesh_half_size.value,
            corner_radii:     record.corner_radii.value,
            border_widths:    record.border_widths.value,
            clip_rect:        record.clip_rect.value,
            fill_material:    record.fill_material.to_gpu(),
            border_material:  record.border_material.to_gpu(),
            paint_mask:       record.paint_mask,
            flags:            record.flags,
            clip_depth_nudge: record.clip_depth_nudge
                - first_draw_order_index.clip_depth_nudge().get(),
            oit_depth_offset: record.oit_depth_offset,
        }
    }

    /// Capacity-tail record that collapses before any material-table read.
    #[must_use]
    const fn padded() -> Self {
        Self {
            transform:        Mat4::ZERO,
            half_size:        Vec2::ZERO,
            mesh_half_size:   Vec2::ZERO,
            corner_radii:     Vec4::ZERO,
            border_widths:    Vec4::ZERO,
            clip_rect:        Vec4::ZERO,
            fill_material:    SdfPaintMaterial::NotAuthored.to_gpu(),
            border_material:  SdfPaintMaterial::NotAuthored.to_gpu(),
            paint_mask:       SdfPaintMask::empty(),
            flags:            0,
            clip_depth_nudge: 0.0,
            oit_depth_offset: 0.0,
        }
    }
}

/// Placeholder SDF mesh table row bound at `SDF_MESH_BINDING`.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
struct SdfMeshRecord {
    /// Reserved data lane for future mesh-level SDF batch metadata.
    reserved: UVec4,
}

const _: () = assert!(SdfRenderRecord::SHADER_SIZE.get() == 160);
const _: () = assert!(SdfMeshRecord::SHADER_SIZE.get() == 16);

/// GPU assets owned by one SDF fill batch.
#[derive(Debug)]
pub(super) struct SdfBatchResources {
    /// `SdfRenderRecord` storage buffer at binding 107.
    pub records:      Handle<ShaderBuffer>,
    /// Reserved SDF mesh-record storage buffer at binding 108.
    pub mesh_records: Handle<ShaderBuffer>,
    /// Inert capacity-sized mesh; replaced on capacity growth.
    pub mesh:         Handle<Mesh>,
    /// Batch render material.
    pub material:     Handle<SdfExtendedMaterial>,
    /// Record capacity represented by `records`, `mesh_records`, and `mesh`.
    pub capacity:     u32,
}

/// CPU record set and GPU handles for one `SdfBatchKey`.
#[derive(Debug, Default)]
pub(super) struct SdfBatch {
    /// Batch render entity; `None` before the first GPU allocation.
    pub entity:             Option<Entity>,
    /// GPU handles; `None` before the first GPU allocation.
    pub gpu:                Option<SdfBatchResources>,
    /// Record-buffer upload state for this batch.
    pub record_upload:      Dirty,
    /// Bounds recomputation state for this batch.
    pub bounds_update:      Dirty,
    /// Deferred sort state for retained records changed in the append pass.
    sort_pending:           Dirty,
    #[cfg(test)]
    sort_count:             usize,
    /// Lowest `DrawOrderIndex` in this batch.
    ///
    /// `SdfRenderRecord::clip_depth_nudge` is uploaded relative to this value;
    /// `SdfRenderRecord::oit_depth_offset` stays panel-absolute.
    first_draw_order_index: DrawOrderIndex,
    records:                Vec<ResolvedSdfBatchRecord>,
}

impl SdfBatch {
    /// CPU records in upload order.
    #[must_use]
    fn records(&self) -> &[ResolvedSdfBatchRecord] { &self.records }

    /// Number of live SDF records in this batch.
    #[must_use]
    fn record_count(&self) -> u32 { self.records.len().to_u32() }

    /// Whether this batch has no live SDF records.
    #[must_use]
    const fn is_empty(&self) -> bool { self.records.is_empty() }

    /// Lowest `DrawOrderIndex` among this batch's live records.
    #[must_use]
    const fn first_draw_order_index(&self) -> DrawOrderIndex { self.first_draw_order_index }

    fn refresh_first_draw_order_index(&mut self) {
        let previous = self.first_draw_order_index;
        self.first_draw_order_index = self
            .records
            .iter()
            .map(|record| record.draw_depth.draw_order_index_value())
            .min()
            .unwrap_or_default();
        if self.first_draw_order_index != previous {
            self.record_upload.mark();
        }
    }

    fn position_of(&self, key: SdfRecordKey) -> Option<usize> {
        self.records
            .iter()
            .position(|record| record.record_key == key)
    }

    fn sort_records(&mut self) {
        #[cfg(test)]
        {
            self.sort_count += 1;
        }
        self.records.sort_by(|left, right| {
            left.draw_depth
                .draw_order_index()
                .cmp(&right.draw_depth.draw_order_index())
                .then(left.draw_order.cmp(&right.draw_order))
        });
    }

    /// Sorts changed records once after the route finishes appending them.
    fn sort_pending_records(&mut self) {
        if !self.sort_pending.is_set() {
            return;
        }
        self.sort_records();
        self.refresh_first_draw_order_index();
        self.sort_pending.clear();
    }

    fn upsert_record(&mut self, mut record: ResolvedSdfBatchRecord) {
        if let Some(position) = self.position_of(record.record_key) {
            // `transform` is the world matrix maintained by
            // `update_sdf_batch_world_transforms`; `from_resolved` only stamps a
            // panel-local placeholder here. Carry the maintained world matrix onto
            // the rebuilt record so an unchanged surface compares equal and skips
            // the record-buffer re-upload instead of failing the check on the
            // local-vs-world matrix difference.
            record.transform = self.records[position].transform;
            if self.records[position] == record {
                return;
            }
            self.records[position] = record;
        } else {
            self.records.push(record);
        }
        self.sort_pending.mark();
        self.record_upload.mark();
        self.bounds_update.mark();
    }

    fn remove_record(&mut self, key: SdfRecordKey) {
        if let Some(position) = self.position_of(key) {
            self.records.remove(position);
            self.refresh_first_draw_order_index();
            self.record_upload.mark();
            self.bounds_update.mark();
        }
    }

    /// World-space union of every live record's clipped mesh corners, each
    /// corner transformed by that record's world matrix.
    #[must_use]
    fn world_bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut min = Vec3::MAX;
        let mut max = Vec3::MIN;
        let mut any = false;
        for record in &self.records {
            let Some((local_min, local_max)) =
                clipped_local_bounds(record.mesh_half_size, record.clip_rect)
            else {
                continue;
            };
            for corner in clipped_corners(local_min, local_max) {
                let world = record.transform * Vec4::new(corner.x, corner.y, 0.0, 1.0);
                min = min.min(world.xyz());
                max = max.max(world.xyz());
                any = true;
            }
        }
        any.then_some((min, max))
    }
}

impl BatchEntry for SdfBatch {
    fn is_empty(&self) -> bool { Self::is_empty(self) }

    fn entity(&self) -> Option<Entity> { self.entity }
}

impl Batch for SdfBatch {
    type MemberKey = SdfRecordKey;
    type Payload = ResolvedSdfBatchRecord;

    fn insert(&mut self, member: Self::MemberKey, payload: Self::Payload) {
        debug_assert_eq!(member, payload.record_key);
        self.upsert_record(payload);
    }

    fn update(&mut self, member: Self::MemberKey, payload: Self::Payload) {
        debug_assert_eq!(member, payload.record_key);
        self.upsert_record(payload);
    }

    fn remove(&mut self, member: Self::MemberKey) { self.remove_record(member); }
}

impl MemberBatch for SdfBatch {
    type Record = ResolvedSdfBatchRecord;

    fn records_mut(&mut self) -> &mut [Self::Record] { &mut self.records }

    fn record_upload_mut(&mut self) -> &mut Dirty { &mut self.record_upload }

    fn bounds_update(&self) -> Dirty { self.bounds_update }

    fn bounds_update_mut(&mut self) -> &mut Dirty { &mut self.bounds_update }

    fn world_bounds(&self) -> Option<(Vec3, Vec3)> { Self::world_bounds(self) }
}

/// Store that maps SDF records to `SdfBatchKey` batches.
#[derive(Debug, Default, Resource)]
pub(super) struct SdfBatchStore(BatchStore<SdfBatchKey, SdfBatch>);

impl SdfBatchStore {
    /// Inserts or moves one retained SDF record.
    fn upsert_record(&mut self, record: ResolvedSdfBatchRecord) {
        let record_key = record.record_key;
        let batch_key = record.batch_key.clone();
        self.0.upsert(batch_key, record_key, record);
    }

    /// Removes every retained SDF record absent from the current append pass.
    fn retain_records(&mut self, active: &HashSet<SdfRecordKey>) { self.0.retain(active); }

    /// All SDF batches.
    fn batches(&self) -> impl Iterator<Item = (&SdfBatchKey, &SdfBatch)> { self.0.batches() }

    /// All SDF batches, mutable.
    fn batches_mut(&mut self) -> impl Iterator<Item = (&SdfBatchKey, &mut SdfBatch)> {
        self.0.batches_mut()
    }

    /// One SDF batch by key, mutable.
    fn get_mut(&mut self, key: &SdfBatchKey) -> Option<&mut SdfBatch> { self.0.get_mut(key) }

    /// Drops empty batch entries, returning their entities for despawn.
    fn take_empty_batches(&mut self) -> Vec<Entity> { self.0.take_empty_batches() }
}

struct SdfMemberFamily;

impl MemberFamily for SdfMemberFamily {
    type Key = SdfBatchKey;
    type Batch = SdfBatch;
    type Store = SdfBatchStore;
    type Marker = DiegeticSdfFillBatch;

    fn store_mut(store: &mut Self::Store) -> &mut BatchStore<Self::Key, Self::Batch> {
        &mut store.0
    }
}

/// How an SDF batch pipeline sources its per-quad geometry.
#[derive(Clone, Copy, Eq, Hash, PartialEq, Debug)]
enum SdfPipelineMode {
    /// Pulls per-quad geometry from the record storage buffer in the vertex
    /// stage (`sdf_panel.wgsl`); the only production mode.
    VertexPulled,
    /// Reads geometry from mesh vertex attributes instead of pulling it.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "non-pulled geometry mode is not produced yet")
    )]
    MeshAttributes,
}

/// SDF material extension over `StandardMaterial`.
#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
#[bind_group_data(SdfExtensionKey)]
pub(crate) struct SdfExtension {
    /// SDF render records read by vertex and fragment stages.
    #[storage(107, read_only, visibility(vertex, fragment))]
    records:        Handle<ShaderBuffer>,
    /// Shared `MaterialSlotValues` table read by SDF fragments.
    #[storage(106, read_only, visibility(fragment))]
    material_table: Handle<ShaderBuffer>,
    /// Reserved SDF mesh metadata table, present for binding-layout parity.
    #[storage(108, read_only, visibility(vertex))]
    mesh_records:   Handle<ShaderBuffer>,
    /// Geometry-sourcing mode for `sdf_panel.wgsl`.
    pipeline_mode:  SdfPipelineMode,
}

/// Pipeline-specialization key for `SdfExtension`.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(crate) struct SdfExtensionKey {
    /// Mirror of `SdfExtension::pipeline_mode`.
    pipeline_mode: SdfPipelineMode,
}

impl From<&SdfExtension> for SdfExtensionKey {
    fn from(extension: &SdfExtension) -> Self {
        Self {
            pipeline_mode: extension.pipeline_mode,
        }
    }
}

impl MaterialExtension for SdfExtension {
    fn vertex_shader() -> ShaderRef { EMBEDDED_SDF_PANEL_BATCH_SHADER_PATH.into() }

    fn fragment_shader() -> ShaderRef { EMBEDDED_SDF_PANEL_BATCH_SHADER_PATH.into() }

    fn prepass_vertex_shader() -> ShaderRef { EMBEDDED_SDF_PANEL_BATCH_SHADER_PATH.into() }

    fn prepass_fragment_shader() -> ShaderRef { EMBEDDED_SDF_PANEL_BATCH_SHADER_PATH.into() }

    fn deferred_vertex_shader() -> ShaderRef { EMBEDDED_SDF_PANEL_BATCH_SHADER_PATH.into() }

    fn deferred_fragment_shader() -> ShaderRef { EMBEDDED_SDF_PANEL_BATCH_SHADER_PATH.into() }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        add_sdf_stripped_material_group_def(descriptor, key.bind_group_data.pipeline_mode);
        Ok(())
    }
}

fn add_sdf_stripped_material_group_def(
    descriptor: &mut RenderPipelineDescriptor,
    pipeline_mode: SdfPipelineMode,
) {
    if matches!(pipeline_mode, SdfPipelineMode::VertexPulled)
        && material_group_is_stripped(descriptor)
    {
        descriptor
            .vertex
            .shader_defs
            .push("SDF_STRIPPED_MATERIAL_GROUP".into());
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment
                .shader_defs
                .push("SDF_STRIPPED_MATERIAL_GROUP".into());
        }
    }
}

fn material_group_is_stripped(descriptor: &RenderPipelineDescriptor) -> bool {
    descriptor
        .layout
        .get(MATERIAL_BIND_GROUP_INDEX)
        .is_none_or(|material_layout| material_layout.entries.is_empty())
}

/// Inputs for one `SdfExtendedMaterial` batch asset.
pub(super) struct SdfBatchMaterialInput {
    /// SDF batch key whose compatibility fields configure the material.
    pub key:          SdfBatchKey,
    /// SDF render-record buffer.
    pub records:      Handle<ShaderBuffer>,
    /// Reserved SDF mesh-record buffer.
    pub mesh_records: Handle<ShaderBuffer>,
}

/// Builds the render material for one SDF batch.
#[must_use]
fn sdf_batch_material(input: SdfBatchMaterialInput) -> SdfExtendedMaterial {
    let SdfBatchMaterialInput {
        key,
        records,
        mesh_records,
    } = input;
    let mut base = batch_key::apply_resource_compatibility_to_standard_material(
        &material::default_panel_material(),
        &key.resource_compatibility,
    );
    base.alpha_mode = sdf_batch_alpha_mode(key.pipeline_compatibility.alpha, key.shadow);
    base.double_sided = key.pipeline_compatibility.double_sided;
    base.cull_mode = key.pipeline_compatibility.cull_mode;
    base.unlit = key.pipeline_compatibility.unlit;
    base.fog_enabled = key.pipeline_compatibility.fog_enabled;
    base.opaque_render_method = key.pipeline_compatibility.opaque_render_method.into();
    base.deferred_lighting_pass_id = key.pipeline_compatibility.deferred_lighting_pass_id;
    base.depth_bias = key.z_index_rank.screen_depth_bias().get();
    ExtendedMaterial {
        base,
        extension: SdfExtension {
            records,
            material_table: Handle::default(),
            mesh_records,
            pipeline_mode: SdfPipelineMode::VertexPulled,
        },
    }
}

/// SDF batch alpha-mode rule for authored source materials.
///
/// Table-row base-color alpha controls normal transparent composition inside
/// `sdf_panel.wgsl`. In prepass/shadow pipelines, `fill_alpha >
/// 0.001` is read from the retained material table when a fill role is present;
/// border-only records skip the fill row and route the transparent interior as
/// empty. `Opaque`, `Mask`, `Blend`, `Premultiplied`, `Add`, `Multiply`, and
/// `AlphaToCoverage` remain stored in `PipelineCompatibility`. A shadow-casting
/// opaque batch maps to `Mask(0.0)` so Bevy keeps the material bind group on the
/// shadow/prepass pipeline (the SDF prepass shader reads the table for
/// `fill_alpha`); a non-casting opaque batch enters no prepass and stays
/// `Opaque`, rendering in the opaque phase like a normal Bevy mesh.
///
/// This differs from text and image batches by construction: text remaps
/// `Opaque` for every batch because the vertex-pull bindings are absent in the
/// depth/normal prepass layout, while image batches always use `Blend`.
#[must_use]
fn sdf_batch_alpha_mode(alpha: BatchAlphaMode, shadow: VisualShadow) -> AlphaMode {
    match (AlphaMode::from(alpha), shadow) {
        (AlphaMode::Opaque, VisualShadow::Cast) => AlphaMode::Mask(0.0),
        (mode, _) => mode,
    }
}

/// Extra `clip_depth_nudge` layer-units to push the SDF fill away from the camera,
/// non-zero only for the depth-buffer regime (`Opaque`/`Mask`). Transparent and
/// OIT fills order through `ScreenDepthBias`/`OitDepthOffset` instead, so they
/// get zero.
#[must_use]
fn opaque_fill_depth_push(alpha: BatchAlphaMode, shadow: VisualShadow) -> f32 {
    match sdf_batch_alpha_mode(alpha, shadow) {
        AlphaMode::Opaque | AlphaMode::Mask(_) => OPAQUE_FILL_DEPTH_PUSH_LAYERS,
        _ => 0.0,
    }
}

/// Repoints an SDF batch material at the current frame material table buffer.
pub(crate) fn set_sdf_material_table_buffer(
    material: &mut SdfExtendedMaterial,
    material_table: Handle<ShaderBuffer>,
) {
    material.extension.material_table = material_table;
}

/// Repoints an SDF batch material at replacement record buffers after growth.
fn set_sdf_material_record_buffers(
    material: &mut SdfExtendedMaterial,
    records: Handle<ShaderBuffer>,
    mesh_records: Handle<ShaderBuffer>,
) {
    material.extension.records = records;
    material.extension.mesh_records = mesh_records;
}

/// Appends fill and border rows for one resolved SDF surface atomically.
fn append_sdf_record_materials(
    builder: &mut FrameMaterialTableBuilder,
    surface: &ResolvedSdfSurface<'_>,
    panel_lighting: Lighting,
    panel_sidedness: Sidedness,
    materials: &Assets<StandardMaterial>,
    asset_server: &AssetServer,
    default_material: &SdfMaterial,
) -> SdfRecordRouting {
    let assignment_checkpoint = builder.assignment_checkpoint();
    let fill = append_sdf_role(
        builder,
        surface,
        SdfMaterialRole::Fill,
        panel_lighting,
        panel_sidedness,
        materials,
        asset_server,
        default_material,
    );
    if let Some(dropped) = SdfRecordRouting::from_dropped_role(&fill) {
        builder.rollback_assignments_after(assignment_checkpoint);
        return dropped;
    }
    let border = append_sdf_role(
        builder,
        surface,
        SdfMaterialRole::Border,
        panel_lighting,
        panel_sidedness,
        materials,
        asset_server,
        default_material,
    );
    if let Some(dropped) = SdfRecordRouting::from_dropped_role(&border) {
        builder.rollback_assignments_after(assignment_checkpoint);
        return dropped;
    }
    match (fill, border) {
        (SdfRoleAppend::NotAuthored, SdfRoleAppend::NotAuthored) => SdfRecordRouting::Unauthored,
        (fill, border) => {
            SdfRecordMaterialSlots::from_appended(fill.into_appended(), border.into_appended())
                .map_or(SdfRecordRouting::Unauthored, SdfRecordRouting::Routed)
        },
    }
}

/// Outcome of appending one resolved SDF surface's fill and border material
/// rows.
///
/// `route_sdf_batch_records` attributes every surface that yields no
/// `SdfRecordMaterialSlots` to a `DroppedSdfSurfaces` counter, so the reasons a
/// surface paints nothing are variants here rather than a bare `None` the
/// caller can discard silently.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum SdfRecordRouting {
    /// Fill and border rows appended; the surface becomes an `SdfBatchStore`
    /// record.
    Routed(SdfRecordMaterialSlots),
    /// Neither `ResolvedSdfSurface::fill_material` nor `border_material` is
    /// authored, so the surface paints nothing.
    Unauthored,
    /// A base `StandardMaterial` is still loading, so
    /// `material::material_asset_for_frame` withheld it for a later frame.
    MaterialPending,
    /// `FrameMaterialTableBuilder` is at row capacity.
    SlotLimit,
}

impl SdfRecordRouting {
    /// Maps an `SdfRoleAppend` that drops its record to the cause, or `None`
    /// when the role appended a row or was unauthored.
    const fn from_dropped_role(role: &SdfRoleAppend) -> Option<Self> {
        match role {
            SdfRoleAppend::Held => Some(Self::MaterialPending),
            SdfRoleAppend::DroppedLimit => Some(Self::SlotLimit),
            SdfRoleAppend::NotAuthored | SdfRoleAppend::Appended(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum SdfRoleAppend {
    NotAuthored,
    Appended(SdfMaterialSlotAppended),
    Held,
    DroppedLimit,
}

#[derive(Clone, Debug, PartialEq)]
struct SdfMaterialSlotAppended {
    /// Stable row assigned by `FrameMaterialTableBuilder`.
    slot:                   MaterialSlotId,
    /// Pipeline compatibility used to choose the batch material.
    pipeline_compatibility: PipelineCompatibility,
    /// Resource compatibility used to create the batch material.
    resource_compatibility: ResourceCompatibility,
}

impl SdfRoleAppend {
    fn into_appended(self) -> Option<SdfMaterialSlotAppended> {
        match self {
            Self::Appended(appended) => Some(appended),
            Self::NotAuthored | Self::Held | Self::DroppedLimit => None,
        }
    }
}

impl SdfRecordMaterialSlots {
    fn from_appended(
        fill: Option<SdfMaterialSlotAppended>,
        border: Option<SdfMaterialSlotAppended>,
    ) -> Option<Self> {
        let fill_material = fill
            .as_ref()
            .map_or(SdfPaintMaterial::NotAuthored, |appended| {
                SdfPaintMaterial::Authored(appended.slot)
            });
        let border_material = border
            .as_ref()
            .map_or(SdfPaintMaterial::NotAuthored, |appended| {
                SdfPaintMaterial::Authored(appended.slot)
            });
        let compatibility = fill.as_ref().or(border.as_ref())?;
        if let (Some(fill), Some(border)) = (&fill, &border) {
            debug_assert_eq!(
                fill.pipeline_compatibility, border.pipeline_compatibility,
                "one SDF record cannot bind two different pipeline policies"
            );
            debug_assert_eq!(
                fill.resource_compatibility, border.resource_compatibility,
                "one SDF record cannot bind two different texture/resource sets"
            );
        }
        Some(Self {
            fill:                   fill_material,
            border:                 border_material,
            pipeline_compatibility: compatibility.pipeline_compatibility,
            resource_compatibility: compatibility.resource_compatibility.clone(),
        })
    }
}

fn append_sdf_role(
    builder: &mut FrameMaterialTableBuilder,
    surface: &ResolvedSdfSurface<'_>,
    role: SdfMaterialRole,
    panel_lighting: Lighting,
    panel_sidedness: Sidedness,
    materials: &Assets<StandardMaterial>,
    asset_server: &AssetServer,
    default_material: &SdfMaterial,
) -> SdfRoleAppend {
    let material = match role {
        SdfMaterialRole::Fill => &surface.fill_material,
        SdfMaterialRole::Border => &surface.border_material,
    };
    if !material.authorship.is_authored() {
        return SdfRoleAppend::NotAuthored;
    }
    let handle = material
        .base_material
        .cloned()
        .unwrap_or_else(|| default_material.0.clone());
    let Some(base_material) =
        material::material_asset_for_frame(materials, asset_server, &handle, &default_material.0)
    else {
        return SdfRoleAppend::Held;
    };
    let input = SdfMaterialSlotInput {
        key: SdfMaterialSourceKey {
            panel: surface.panel_entity,
            element: surface.element_index,
            divider_ordinal: surface.divider_ordinal,
            role,
        },
        base_material,
        color_override: material.color,
        lighting: panel_lighting,
        sidedness: panel_sidedness,
    };
    match material_table::append_material_slot(builder, &input) {
        MaterialSlotAppend::Appended(appended) => {
            SdfRoleAppend::Appended(SdfMaterialSlotAppended {
                slot:                   appended.slot,
                pipeline_compatibility: appended.pipeline_compatibility,
                resource_compatibility: appended.resource_compatibility,
            })
        },
        MaterialSlotAppend::DroppedLimit => SdfRoleAppend::DroppedLimit,
    }
}

/// Assigns contiguous run ordinals independently for each `BatchRenderLayers`.
fn assign_contiguous_runs(
    records: Vec<(SdfRecordMaterialSlots, &ResolvedSdfSurface<'_>)>,
) -> Vec<ResolvedSdfBatchRecord> {
    let record_count = records.len();
    let mut by_layers: HashMap<BatchRenderLayers, Vec<_>> = HashMap::new();
    for (materials, surface) in records {
        by_layers
            .entry(BatchRenderLayers(surface.render_layers.clone()))
            .or_default()
            .push((materials, surface));
    }

    let mut output = Vec::with_capacity(record_count);
    for (_, mut partition) in by_layers {
        partition.sort_by(|(_, left), (_, right)| {
            left.draw_depth
                .draw_order_index()
                .cmp(&right.draw_depth.draw_order_index())
                .then(left.command_index.cmp(&right.command_index))
        });

        let mut previous = None;
        let mut run = ContiguousDrawnRun::default();
        for (materials, surface) in partition {
            let compatibility = SdfRunCompatibility::from_surface(surface, &materials);
            if previous
                .as_ref()
                .is_some_and(|previous| previous != &compatibility)
            {
                run.value = run.value.saturating_add(1);
            }
            previous = Some(compatibility);
            output.push(ResolvedSdfBatchRecord::from_resolved(
                surface, materials, run,
            ));
        }
    }
    output
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SdfRunCompatibility {
    z_index:                DrawZIndex,
    z_index_rank:           DrawZIndexRank,
    layers:                 BatchRenderLayers,
    shadow:                 VisualShadow,
    pipeline_compatibility: PipelineCompatibility,
    resource_compatibility: ResourceCompatibility,
}

impl SdfRunCompatibility {
    fn from_surface(surface: &ResolvedSdfSurface<'_>, materials: &SdfRecordMaterialSlots) -> Self {
        Self {
            z_index:                surface.draw_depth.z_index(),
            z_index_rank:           surface.draw_depth.z_index_rank(),
            layers:                 BatchRenderLayers(surface.render_layers.clone()),
            shadow:                 surface.shadow_casting.into(),
            pipeline_compatibility: sdf_record_pipeline_compatibility(surface, materials),
            resource_compatibility: materials.resource_compatibility.clone(),
        }
    }
}

/// Render plugin for the SDF fill batch material type.
pub(super) struct FillBatchPlugin;

impl Plugin for FillBatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SdfBatchStore>()
            .init_resource::<DiegeticPerfStats>()
            .init_resource::<SdfRecordDiagnostics>()
            .init_resource::<VisualOverrideIndex>()
            .add_plugins(MaterialPlugin::<SdfExtendedMaterial>::default())
            .add_systems(
                PostUpdate,
                route_sdf_batch_records
                    .after(material_table::clear_frame_material_table)
                    .in_set(MaterialTableAppendReady),
            )
            .add_systems(
                PostUpdate,
                update_sdf_batch_world_transforms.after(TransformSystems::Propagate),
            )
            .add_systems(
                PostUpdate,
                reconcile_sdf_batch_entities
                    .after(update_sdf_batch_world_transforms)
                    .in_set(BatchAssetWrites)
                    .before(VisibilitySystems::CalculateBounds)
                    .in_set(BatchResourcesReady),
            )
            .add_systems(
                PostUpdate,
                material_table::register_sdf_batch_materials::<DiegeticSdfFillBatch>
                    .after(reconcile_sdf_batch_entities)
                    .in_set(BatchResourcesReady),
            )
            .add_systems(
                PostUpdate,
                update_sdf_batch_bounds
                    .after(material_table::register_sdf_batch_materials::<DiegeticSdfFillBatch>)
                    .after(VisibilitySystems::CalculateBounds)
                    .before(VisibilitySystems::CheckVisibility)
                    .in_set(BatchResourcesReady),
            )
            .add_systems(
                PostUpdate,
                commit_sdf_batch_buffers
                    .after(material_table::register_sdf_batch_materials::<DiegeticSdfFillBatch>)
                    .before(update_sdf_batch_bounds)
                    .in_set(BatchResourcesReady),
            );
    }
}

fn route_sdf_batch_records(
    #[cfg(test)] mut run_order: Option<ResMut<SdfDriverRunOrder>>,
    mut build: ResMut<material_table::FrameMaterialTableBuild>,
    mut surfaces: ResMut<ResolvedSdfSurfaceRegistry>,
    standard_materials: Res<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    sdf_material_default: Res<SdfMaterial>,
    lighting_default: Res<Lighting>,
    sidedness_default: Res<Sidedness>,
    visual_overrides: Res<VisualOverrideIndex>,
    panels: Query<(
        &DiegeticPanel,
        Option<&RenderLayers>,
        Option<&Visibility>,
        Option<&Resolved<Lighting>>,
        Option<&Resolved<Sidedness>>,
        Option<&Resolved<ShadowCasting>>,
    )>,
    mut store: ResMut<SdfBatchStore>,
    mut perf: ResMut<DiegeticPerfStats>,
) {
    #[cfg(test)]
    material_table::record_sdf_driver_run(&mut run_order, material_table::SDF_DRIVER_ROUTE_RESOLVE);

    let mut resolved_surfaces = Vec::new();
    let mut active_records = HashSet::new();
    let mut stale_panels = HashSet::new();
    let mut dropped = DroppedSdfSurfaces::default();
    let mut surface_count = 0_usize;
    for stored_surface in surfaces.surfaces() {
        surface_count += 1;
        let Ok((
            _panel,
            panel_layers,
            panel_visibility,
            resolved_lighting,
            resolved_sidedness,
            resolved_shadow_casting,
        )) = panels.get(stored_surface.panel_entity())
        else {
            // `panel_missing` is counted by the eviction below, from the length
            // of the bucket `remove_panel` drops. Counting it here as well would
            // report every surface twice.
            stale_panels.insert(stored_surface.panel_entity());
            continue;
        };
        // Hidden panels keep their frame-table rows and SDF batch records so a
        // `Visibility` change updates the existing GPU buffer allocation.
        let panel_visibility = panel_visibility.copied().unwrap_or(Visibility::Inherited);
        let panel_lighting = resolved_lighting.map_or(*lighting_default, |resolved| resolved.0);
        let panel_sidedness = resolved_sidedness.map_or(*sidedness_default, |resolved| resolved.0);
        let panel_shadow_casting =
            resolved_shadow_casting.map_or(ShadowCasting::On, |resolved| resolved.0);
        let mut resolved = stored_surface.as_resolved(
            panel_layers.cloned().unwrap_or(RenderLayers::layer(0)),
            panel_shadow_casting,
        );
        if let Some(slot_override) =
            stored_surface
                .visual_source_element()
                .and_then(|element_index| {
                    visual_overrides.get(stored_surface.panel_entity(), element_index.get())
                })
        {
            apply_sdf_visual_override(&mut resolved, slot_override);
        }
        resolved_surfaces.push((resolved, panel_lighting, panel_sidedness, panel_visibility));
    }
    let mut appended = Vec::new();
    let mut hidden_records = HashSet::new();
    let builder = build.builder_mut();
    for (surface, panel_lighting, panel_sidedness, panel_visibility) in &resolved_surfaces {
        let record_key = SdfRecordKey {
            panel:           surface.panel_entity,
            element:         surface.element_index,
            divider_ordinal: surface.divider_ordinal,
            role:            surface.record_role,
        };
        let routing = append_sdf_record_materials(
            builder,
            surface,
            *panel_lighting,
            *panel_sidedness,
            &standard_materials,
            &asset_server,
            &sdf_material_default,
        );
        let materials = match routing {
            SdfRecordRouting::Routed(materials) => materials,
            SdfRecordRouting::Unauthored => {
                dropped.unauthored += 1;
                continue;
            },
            SdfRecordRouting::MaterialPending => {
                dropped.material_pending += 1;
                continue;
            },
            SdfRecordRouting::SlotLimit => {
                dropped.slot_limit += 1;
                continue;
            },
        };
        if *panel_visibility == Visibility::Hidden {
            hidden_records.insert(record_key);
        }
        active_records.insert(record_key);
        appended.push((materials, surface));
    }

    for mut record in assign_contiguous_runs(appended) {
        if hidden_records.contains(&record.record_key) {
            record.mark_hidden();
        }
        store.upsert_record(record);
    }
    store.retain_records(&active_records);
    for (_, batch) in store.batches_mut() {
        batch.sort_pending_records();
    }
    drop(resolved_surfaces);
    for panel_entity in stale_panels {
        surfaces.remove_panel(panel_entity, SurfaceEvictionCause::PanelMissing);
    }
    // Evictions join this frame's tally before it reaches perf, so a drop
    // recorded by `build_panel_geometry` earlier in the frame is counted here
    // rather than lost.
    let evictions = surfaces.take_evictions();
    dropped.layout_unsolved += evictions.layout_unsolved.get();
    dropped.panel_missing += evictions.panel_missing.get();
    perf.panel_geometry.resolved_surfaces = surface_count.into();
    perf.panel_geometry.dropped_surfaces.accumulate(&dropped);
}

/// Applies one widget visual-slot override to a routed SDF surface.
///
/// Runs inside `route_sdf_batch_records`, so the override reaches the same
/// frame material rows, `SdfBatchKey` selection, and `SdfBatchStore` upsert
/// as authored data: `fill_color` and `border_color` patch their authored
/// material rows in place. Replacement border widths restate the record's
/// inward inset without moving its outer bounds, an
/// offset translates only its `local_transform`, and a replacement material
/// that changes `PipelineCompatibility`/`ResourceCompatibility` re-keys the
/// record to its compatible destination batch. Authored registry surfaces
/// stay untouched, and every branch checks `SdfRoleAuthorship` first:
/// overrides never author a missing fill or border role.
fn apply_sdf_visual_override<'a>(
    surface: &mut ResolvedSdfSurface<'a>,
    slot_override: &'a VisualSlotOverride,
) {
    let fill_authored = surface.fill_material.authorship.is_authored();
    let border_authored = surface.border_material.authorship.is_authored();
    if let Some(color) = slot_override.fill_color
        && fill_authored
    {
        surface.fill_material.color = Some(color);
    }
    if let Some(color) = slot_override.border_color
        && border_authored
    {
        surface.border_material.color = Some(color);
    }
    if let Some(border_widths) = slot_override.border_widths
        && border_authored
    {
        surface.border_widths = border_widths;
    }
    if let Some(material) = &slot_override.material {
        if fill_authored {
            surface.fill_material.base_material = Some(material);
        }
        if border_authored {
            surface.border_material.base_material = Some(material);
        }
    }
    if let Some(offset) = slot_override.offset {
        surface.local_transform.translation += offset.extend(0.0);
    }
}

fn update_sdf_batch_world_transforms(
    #[cfg(test)] mut run_order: Option<ResMut<SdfDriverRunOrder>>,
    store: ResMut<SdfBatchStore>,
    panel_transforms: Query<&GlobalTransform, With<DiegeticPanel>>,
) {
    #[cfg(test)]
    material_table::record_sdf_driver_run(
        &mut run_order,
        material_table::SDF_DRIVER_WORLD_TRANSFORMS,
    );

    batch_store::update_batch_world_transforms::<SdfMemberFamily>(store, panel_transforms);
}

fn reconcile_sdf_batch_entities(
    #[cfg(test)] mut run_order: Option<ResMut<SdfDriverRunOrder>>,
    mut store: ResMut<SdfBatchStore>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SdfExtendedMaterial>>,
    mut storage_buffers: ResMut<Assets<ShaderBuffer>>,
    mut commands: Commands,
) {
    #[cfg(test)]
    material_table::record_sdf_driver_run(
        &mut run_order,
        material_table::SDF_DRIVER_RECONCILE_SPAWN,
    );

    for entity in store.take_empty_batches() {
        commands.entity(entity).despawn();
    }

    let mut to_create = Vec::new();
    let mut to_grow = Vec::new();
    for (key, batch) in store.batches() {
        match &batch.gpu {
            None => to_create.push(key.clone()),
            Some(gpu) if batch.record_count() > gpu.capacity => to_grow.push(key.clone()),
            Some(_) => {},
        }
    }

    for key in to_create {
        let Some(batch) = store.get_mut(&key) else {
            continue;
        };
        spawn_sdf_batch_entity(
            &key,
            batch,
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut storage_buffers,
        );
    }
    for key in to_grow {
        let Some(batch) = store.get_mut(&key) else {
            continue;
        };
        grow_sdf_batch_assets(
            batch,
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut storage_buffers,
        );
    }
    refresh_sdf_batch_material_depth_biases(&mut store, &mut materials);
}

fn refresh_sdf_batch_material_depth_biases(
    store: &mut SdfBatchStore,
    materials: &mut Assets<SdfExtendedMaterial>,
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

fn padded_sdf_render_records(
    records: &[ResolvedSdfBatchRecord],
    first_draw_order_index: DrawOrderIndex,
    capacity: u32,
) -> Vec<SdfRenderRecord> {
    let mut padded = Vec::with_capacity(capacity.to_usize());
    padded.extend(
        records
            .iter()
            .map(|record| SdfRenderRecord::from_resolved(record, first_draw_order_index)),
    );
    padded.resize(
        capacity.to_usize().max(records.len()),
        SdfRenderRecord::padded(),
    );
    padded
}

fn padded_sdf_mesh_records(capacity: u32) -> Vec<SdfMeshRecord> {
    vec![SdfMeshRecord::default(); capacity.to_usize()]
}

fn inert_sdf_batch_mesh(capacity: u32) -> Mesh {
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

fn clipped_local_bounds(
    mesh_half_size: MeshHalfSize,
    clip_rect: LocalClipRect,
) -> Option<(Vec2, Vec2)> {
    let mesh_min = -mesh_half_size.value;
    let mesh_max = mesh_half_size.value;
    let clip_min = Vec2::new(clip_rect.value.x, clip_rect.value.y);
    let clip_max = Vec2::new(clip_rect.value.z, clip_rect.value.w);
    let min = mesh_min.max(clip_min);
    let max = mesh_max.min(clip_max);
    (min.x < max.x && min.y < max.y).then_some((min, max))
}

const fn clipped_corners(min: Vec2, max: Vec2) -> [Vec2; 4] {
    [
        Vec2::new(min.x, max.y),
        Vec2::new(max.x, max.y),
        Vec2::new(max.x, min.y),
        Vec2::new(min.x, min.y),
    ]
}

/// Spawns an SDF batch entity and its GPU assets.
fn spawn_sdf_batch_entity(
    key: &SdfBatchKey,
    batch: &mut SdfBatch,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<SdfExtendedMaterial>,
    storage_buffers: &mut Assets<ShaderBuffer>,
) {
    let capacity = batch.record_count().max(1).next_power_of_two();
    let records = storage_buffers.add(ShaderBuffer::from(padded_sdf_render_records(
        batch.records(),
        batch.first_draw_order_index(),
        capacity,
    )));
    let mesh_records = storage_buffers.add(ShaderBuffer::from(padded_sdf_mesh_records(capacity)));
    let mesh = meshes.add(inert_sdf_batch_mesh(capacity));
    let material = materials.add(sdf_batch_material(SdfBatchMaterialInput {
        key:          key.clone(),
        records:      records.clone(),
        mesh_records: mesh_records.clone(),
    }));
    let mut entity = commands.spawn((
        DiegeticSdfFillBatch,
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material.clone()),
        Pickable::IGNORE,
        Visibility::Inherited,
        NoAutoAabb,
        Aabb::default(),
        key.layers.0.clone(),
    ));
    if key.shadow == VisualShadow::None {
        entity.insert(NotShadowCaster);
    }
    batch.entity = Some(entity.id());
    batch.gpu = Some(SdfBatchResources {
        records,
        mesh_records,
        mesh,
        material,
        capacity,
    });
    batch.record_upload.clear();
}

/// Grows one SDF batch's buffers and inert mesh to fit its live records.
fn grow_sdf_batch_assets(
    batch: &mut SdfBatch,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<SdfExtendedMaterial>,
    storage_buffers: &mut Assets<ShaderBuffer>,
) {
    let Some(entity) = batch.entity else {
        return;
    };
    let Some(current_capacity) = batch.gpu.as_ref().map(|gpu| gpu.capacity) else {
        return;
    };
    let required = batch.record_count().max(1);
    if required <= current_capacity {
        return;
    }
    let mut capacity = current_capacity.max(1);
    while capacity < required {
        capacity *= 2;
    }
    let records = storage_buffers.add(ShaderBuffer::from(padded_sdf_render_records(
        batch.records(),
        batch.first_draw_order_index(),
        capacity,
    )));
    let mesh_records = storage_buffers.add(ShaderBuffer::from(padded_sdf_mesh_records(capacity)));
    let mesh = meshes.add(inert_sdf_batch_mesh(capacity));
    commands.entity(entity).insert(Mesh3d(mesh.clone()));
    let Some(gpu) = batch.gpu.as_mut() else {
        return;
    };
    if let Some(mut material) = materials.get_mut(&gpu.material) {
        set_sdf_material_record_buffers(&mut material, records.clone(), mesh_records.clone());
    }
    gpu.records = records;
    gpu.mesh_records = mesh_records;
    gpu.mesh = mesh;
    gpu.capacity = capacity;
    batch.record_upload.clear();
}

/// Updates SDF batch entity placement and local `Aabb` from record bounds.
fn update_sdf_batch_bounds(
    #[cfg(test)] mut run_order: Option<ResMut<SdfDriverRunOrder>>,
    store: ResMut<SdfBatchStore>,
    batch_entities: Query<
        (&mut Transform, &mut GlobalTransform, &mut Aabb),
        With<DiegeticSdfFillBatch>,
    >,
) {
    #[cfg(test)]
    material_table::record_sdf_driver_run(&mut run_order, material_table::SDF_DRIVER_BOUNDS);

    batch_store::update_batch_bounds::<SdfMemberFamily>(store, batch_entities);
}

/// Uploads dirty SDF record buffers with fixed-capacity payloads.
fn commit_sdf_batch_buffers(
    #[cfg(test)] mut run_order: Option<ResMut<SdfDriverRunOrder>>,
    mut store: ResMut<SdfBatchStore>,
    mut staged_uploads: ResMut<RetainedBatchBufferUploads>,
    mut perf: ResMut<DiegeticPerfStats>,
    mut diagnostics: ResMut<SdfRecordDiagnostics>,
) {
    #[cfg(test)]
    material_table::record_sdf_driver_run(&mut run_order, material_table::SDF_DRIVER_COMMIT);

    let mut uploads = 0_usize;
    let mut batches = 0_usize;
    let mut records = 0_usize;
    diagnostics.records.clear();
    diagnostics.batches.clear();
    perf.sdf_breakdown.clear();
    for (key, batch) in store.batches_mut() {
        batches += 1;
        perf.sdf_breakdown.push(super::batch_summary(
            key.z_index,
            &key.layers,
            key.shadow,
            &key.pipeline_compatibility,
            &key.resource_compatibility,
            batch.record_count(),
        ));
        diagnostics.batches.push(SdfBatchSummary {
            z_index:                key.z_index,
            render_layers:          key.layers.0.iter().map(usize::to_u32).collect(),
            shadow:                 format!("{:?}", key.shadow),
            contiguous_run:         key.contiguous_drawn_run.value,
            record_count:           batch.record_count(),
            pipeline_compatibility: format!("{:?}", key.pipeline_compatibility),
            resource_compatibility: format!("{:?}", key.resource_compatibility),
        });
        records += batch.records().len();
        for resolved in batch.records() {
            let record = SdfRenderRecord::from_resolved(resolved, batch.first_draw_order_index());
            diagnostics.records.push(SdfRecordSnapshot {
                fill_material:   record.fill_material.as_u32(),
                border_material: record.border_material.as_u32(),
                paint_mask:      record.paint_mask.bits(),
                half_size:       record.half_size,
                mesh_half_size:  record.mesh_half_size,
                flags:           record.flags,
            });
        }
        if !batch.record_upload.is_set() {
            continue;
        }
        let Some(gpu) = &batch.gpu else {
            continue;
        };
        let payload = padded_sdf_render_records(
            batch.records(),
            batch.first_draw_order_index(),
            gpu.capacity,
        );
        batch.record_upload.clear();
        material_table::stage_retained_batch_buffer_upload(
            &mut staged_uploads,
            &gpu.records,
            ShaderBuffer::from(payload),
        );
        uploads += 1;
    }
    perf.panel_geometry.sdf_batches = batches.into();
    perf.panel_geometry.sdf_records = records.into();
    perf.panel_geometry.sdf_uploads = uploads.into();
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should fail loudly when SDF batch fixtures are invalid"
)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::ops::Range;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use bevy::asset::Asset;
    use bevy::asset::AssetEvent;
    use bevy::asset::AssetPlugin;
    use bevy::camera::visibility::RenderLayers;
    use bevy::ecs::change_detection::Tick;
    use bevy::ecs::message::Messages;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::image::Image;
    use bevy::picking::hover::PickingInteraction;
    use bevy::prelude::AlphaMode;
    use bevy::prelude::AssetApp;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::TransformPlugin;
    use bevy::prelude::UVec2;
    use bevy::prelude::Window;
    use bevy::prelude::default;
    use bevy::render::render_resource::Face;
    use bevy::render::render_resource::FragmentState;
    use bevy::shader::Shader;
    use bevy::shader::ShaderDefVal;
    use bevy::window::PrimaryWindow;
    use hana_kana::ToF32;

    use super::*;
    use crate::AlignX;
    use crate::AlignY;
    use crate::BorderColor;
    use crate::Mm;
    use crate::Padding;
    use crate::PanelElementId;
    use crate::PanelWidget;
    use crate::PanelWidgetReader;
    use crate::Px;
    use crate::Slider;
    use crate::SliderDirection;
    use crate::SliderState;
    use crate::Tooltip;
    use crate::TooltipPlacementPolicy;
    use crate::Tooltips;
    use crate::layout::Border;
    use crate::layout::BoundingBox;
    use crate::layout::ChildDivider;
    use crate::layout::DrawZIndex;
    use crate::layout::El;
    use crate::layout::LayoutBuilder;
    use crate::layout::LayoutTree;
    use crate::layout::RectangleSource;
    use crate::layout::RenderCommand;
    use crate::layout::RenderCommandKind;
    use crate::layout::ShadowCasting;
    use crate::layout::Sizing;
    use crate::layout::TextDimensions;
    use crate::layout::TextMeasure;
    use crate::layout::TextStyle;
    use crate::panel::ComputedDiegeticPanel;
    use crate::panel::DiegeticPanelCommands;
    use crate::panel::HeadlessLayoutPlugin;
    use crate::panel::LifetimeTotal;
    use crate::render::PathExtendedMaterial;
    use crate::render::image_batch::ImageBatchPlugin;
    use crate::render::image_batch::ImageBatchStore;
    use crate::render::image_batch::ResolvedImageRecord;
    use crate::render::material_table::FrameMaterialSlotAppend;
    use crate::render::material_table::FrameMaterialTableBuild;
    use crate::render::material_table::INVALID_GPU_MATERIAL_SLOT;
    use crate::render::material_table::MATERIAL_TABLE_BINDING;
    use crate::render::material_table::MaterialSlotId;
    use crate::render::material_table::MaterialSlotValues;
    use crate::render::material_table::MaterialTableBuffer;
    use crate::render::material_table::MaterialTablePlugin;
    use crate::render::material_table::SDF_MESH_BINDING;
    use crate::render::material_table::SDF_RENDER_RECORDS_BINDING;
    use crate::render::panel_geometry::PanelGeometryPlugin;
    use crate::render::panel_geometry::ResolvedSdfMaterial;
    use crate::render::panel_geometry::ResolvedSdfSurface;
    use crate::render::panel_geometry::SdfRoleAuthorship;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::ComputedWidgetRecord;
    use crate::widgets::VisualOverrideIndex;
    use crate::widgets::VisualSlotId;
    use crate::widgets::VisualSlotOverride;
    use crate::widgets::WidgetAnchorRect;
    use crate::widgets::WidgetVisualOverrides;
    use crate::widgets::WidgetVisualSlots;
    use crate::widgets::WidgetsPlugin;

    const ANIMATED_BASE_BLUE_SPEED: f32 = 0.29;
    const ANIMATED_BASE_COLOR_SWING: f32 = 0.05;
    const ANIMATED_BASE_GREEN_SPEED: f32 = 0.23;
    const ANIMATED_BASE_RED_SPEED: f32 = 0.19;
    const CHILD_DIVIDER_COLOR: Color = Color::srgb(0.1, 0.8, 0.4);
    const DIVIDER_PANEL_METALLIC: f32 = 0.15;
    const IMAGE_BORDER_COLOR: Color = Color::srgb(0.9, 0.8, 0.2);
    const IMAGE_BORDER_WIDTH_MM: f32 = 1.0;
    const OVERRIDE_OFFSET: Vec2 = Vec2::new(0.005, -0.002);
    const PARENT_ELEMENT_CORNER_RADIUS: f32 = 4.0;
    const PARENT_ELEMENT_METALLIC: f32 = 0.85;
    const PEER_FILL_COLOR: Color = Color::srgb(0.2, 0.3, 0.8);
    const SDF_SETTLE_FRAMES: usize = 3;
    const STATE_BORDER_COLOR: Color = Color::srgb(0.9, 0.7, 0.2);
    const SLOT_OVERRIDE_COLOR: Color = Color::srgb(0.1, 0.9, 0.2);
    const TEST_SLOT: VisualSlotId = VisualSlotId::new(1);
    const TOOLTIP_REVEAL_SETTLE_FRAMES: usize = 6;
    const WIDGET_FILL_COLOR: Color = Color::srgb(0.8, 0.2, 0.1);
    const WIDGET_LABEL_FILL_COLOR: Color = Color::srgb(0.7, 0.2, 0.4);
    const WIDGET_LABEL_SLOT: VisualSlotId = VisualSlotId::new(2);
    const WIDGET_THUMB_FILL_COLOR: Color = Color::srgb(0.2, 0.7, 0.4);
    const WIDGET_THUMB_SLOT: VisualSlotId = VisualSlotId::new(4);
    const WIDGET_TRACK_FILL_COLOR: Color = Color::srgb(0.2, 0.4, 0.7);
    const WIDGET_TRACK_SLOT: VisualSlotId = VisualSlotId::new(3);

    fn zero_measurer() -> DiegeticTextMeasurer {
        DiegeticTextMeasurer {
            measure_fn: Arc::new(|_: &str, measure: &TextMeasure| TextDimensions {
                width:       0.0,
                height:      measure.size,
                line_height: measure.size,
            }),
        }
    }

    fn sdf_pipeline_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .add_plugins(TransformPlugin)
            .insert_resource(zero_measurer())
            .add_plugins(HeadlessLayoutPlugin)
            .init_asset::<Mesh>()
            .init_asset::<Shader>()
            .init_asset::<ShaderBuffer>()
            .init_asset::<StandardMaterial>()
            .init_asset::<PathExtendedMaterial>()
            .init_asset::<SdfExtendedMaterial>()
            .add_plugins(crate::cascade::cascade_plugin::<SdfMaterial>())
            .add_plugins((MaterialTablePlugin, FillBatchPlugin, PanelGeometryPlugin));
        let default_material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(material::default_panel_material());
        app.insert_resource(SdfMaterial(default_material));
        app.insert_resource(Lighting::Lit);
        app.insert_resource(Sidedness::BothSides);
        app
    }

    fn image_sdf_pipeline_app() -> App {
        let mut app = sdf_pipeline_app();
        app.init_asset::<Image>().add_plugins(ImageBatchPlugin);
        app
    }

    fn settle_sdf_pipeline(app: &mut App) {
        for _ in 0..SDF_SETTLE_FRAMES {
            app.update();
        }
    }

    fn spawn_sdf_panel(app: &mut App, tree: LayoutTree, material: StandardMaterial) -> Entity {
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(material);
        app.world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .material(material)
                    .with_tree(tree)
                    .build()
                    .expect("panel should build"),
            )
            .id()
    }

    fn spawn_sdf_screen_panel(
        app: &mut App,
        tree: LayoutTree,
        material: StandardMaterial,
        window: Entity,
        camera_order: isize,
        render_layers: RenderLayers,
    ) -> Entity {
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(material);
        app.world_mut()
            .spawn(
                DiegeticPanel::screen()
                    .size(Px(320.0), Px(180.0))
                    .window_entity(window)
                    .camera_order(camera_order)
                    .render_layers(render_layers)
                    .material(material)
                    .with_tree(tree)
                    .build()
                    .expect("screen panel should build"),
            )
            .id()
    }

    fn tooltip_surface_tree(label: &'static str) -> LayoutTree {
        let mut tooltip = Tooltip::new(
            El::column()
                .width(Sizing::FIT)
                .height(Sizing::FIT)
                .padding(Padding::all(2.0))
                .background(Color::srgb(0.2, 0.8, 0.4)),
        )
        .show_after(Duration::ZERO)
        .hide_after(Duration::ZERO)
        .placement_policy(TooltipPlacementPolicy::Fixed);
        tooltip.text((label, TextStyle::new(4.0)));
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::new()
                .size(40.0, 20.0)
                .background(WIDGET_FILL_COLOR)
                .button("tooltip-target")
                .tooltip(tooltip),
            |_| {},
        );
        builder.build()
    }

    fn one_surface_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(10.0), Mm(10.0));
        builder.with(
            El::new()
                .size(10.0, 10.0)
                .background(Color::srgb(0.1, 0.2, 0.3)),
            |_| {},
        );
        builder.build()
    }

    fn assert_tooltip_surface_is_retained(app: &App, controller: Entity) {
        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(controller)
            .expect("tooltip should complete layout");
        assert!(computed.is_solved(), "tooltip layout should succeed");
        assert!(
            computed
                .layout()
                .solved()
                .is_some_and(|result| !result.commands.is_empty()),
            "tooltip layout should contain render commands: {:?}",
            computed.layout().solved().map(|result| &result.commands),
        );
        let tooltip_surfaces = app
            .world()
            .resource::<ResolvedSdfSurfaceRegistry>()
            .surfaces()
            .filter(|surface| surface.panel_entity() == controller)
            .count();
        let tooltip_panel = app
            .world()
            .get::<DiegeticPanel>(controller)
            .expect("tooltip should retain its panel role");
        assert!(
            tooltip_surfaces > 0,
            "the tooltip should publish SDF surfaces; size: {}x{}; tree: {:?}; result: {:?}",
            tooltip_panel.width(),
            tooltip_panel.height(),
            tooltip_panel.tree(),
            computed.layout(),
        );
        let retained_panels = sdf_records(app)
            .iter()
            .map(|record| record.record_key.panel)
            .collect::<Vec<_>>();
        assert!(
            sdf_records(app)
                .iter()
                .any(|record| record.record_key.panel == controller),
            "the reveal should route the tooltip surface; retained panels: {retained_panels:?}",
        );
    }

    fn assert_tooltip_retained_transform(app: &App, controller: Entity) {
        let record = sdf_records(app)
            .into_iter()
            .find(|record| record.record_key.panel == controller)
            .expect("a ready tooltip should have a retained record");
        let global = app
            .world()
            .get::<GlobalTransform>(controller)
            .expect("revealed tooltip should have a propagated transform");
        assert_eq!(
            record.transform,
            global.to_matrix() * record.local_transform.to_matrix(),
            "the retained record must contain the reveal frame's final transform",
        );
    }

    struct ShownTooltipFixture {
        app:        App,
        panel:      Entity,
        widget:     Entity,
        controller: Entity,
    }

    #[derive(Clone, Debug, PartialEq)]
    struct SdfBatchGpuIdentity {
        key:          SdfBatchKey,
        entity:       Entity,
        records:      Handle<ShaderBuffer>,
        mesh_records: Handle<ShaderBuffer>,
        mesh:         Handle<Mesh>,
        capacity:     u32,
    }

    fn sdf_batch_gpu_identity_for_panel(app: &App, panel: Entity) -> SdfBatchGpuIdentity {
        let store = app.world().resource::<SdfBatchStore>();
        let (key, batch) = store
            .batches()
            .find(|(_, batch)| {
                batch
                    .records()
                    .iter()
                    .any(|record| record.record_key.panel == panel)
            })
            .expect("panel should belong to an SDF batch");
        let gpu = batch
            .gpu
            .as_ref()
            .expect("the settled SDF batch should own GPU assets");
        SdfBatchGpuIdentity {
            key:          key.clone(),
            entity:       batch
                .entity
                .expect("the settled SDF batch should own an entity"),
            records:      gpu.records.clone(),
            mesh_records: gpu.mesh_records.clone(),
            mesh:         gpu.mesh.clone(),
            capacity:     gpu.capacity,
        }
    }

    fn shown_tooltip_fixture() -> ShownTooltipFixture {
        let mut app = sdf_pipeline_app();
        app.add_plugins(WidgetsPlugin);
        let panel = spawn_sdf_panel(
            &mut app,
            tooltip_surface_tree("Retained tooltip"),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        let widget = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named("tooltip-target"))
            })
            .ok()
            .flatten()
            .expect("tooltip target widget should reify");
        let controller = app
            .world()
            .get::<Tooltips>(widget)
            .and_then(|tooltips| tooltips.iter().next())
            .expect("tooltip controller should reify");
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        for _ in 0..TOOLTIP_REVEAL_SETTLE_FRAMES {
            app.update();
        }
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Inherited),
        );
        assert_tooltip_surface_is_retained(&app, controller);
        ShownTooltipFixture {
            app,
            panel,
            widget,
            controller,
        }
    }

    #[test]
    fn first_tooltip_reveal_preserves_peer_rows_and_material_table_allocation() {
        let mut app = sdf_pipeline_app();
        app.add_plugins(WidgetsPlugin);
        let panel = spawn_sdf_panel(
            &mut app,
            tooltip_surface_tree("First tooltip"),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        let widget = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named("tooltip-target"))
            })
            .ok()
            .flatten()
            .expect("tooltip target widget should reify");
        let controller = app
            .world()
            .get::<Tooltips>(widget)
            .and_then(|tooltips| tooltips.iter().next())
            .expect("tooltip controller should reify");

        let initial_capacity = app.world().resource::<MaterialTableBuffer>().capacity;
        let initial_rows = app
            .world()
            .resource::<FrameMaterialTableBuild>()
            .table()
            .live_row_count();
        let rows_before_old_growth_boundary = initial_capacity
            .saturating_mul(3)
            .checked_div(4)
            .unwrap_or(0)
            .saturating_sub(1);
        let filler_count = rows_before_old_growth_boundary.saturating_sub(initial_rows.to_u32());
        for _ in 0..filler_count {
            spawn_sdf_panel(&mut app, one_surface_tree(), StandardMaterial::default());
        }
        settle_sdf_pipeline(&mut app);

        let table_buffer = app.world().resource::<MaterialTableBuffer>();
        let table_handle = table_buffer
            .handle
            .clone()
            .expect("the settled table should have an allocated buffer");
        let table_capacity = table_buffer.capacity;
        let table_allocations = table_buffer.allocations;
        assert_eq!(table_capacity, initial_capacity);
        let peer_materials = sdf_material_assignments_except(&app, controller);
        assert!(
            sdf_records(&app)
                .iter()
                .all(|record| record.record_key.panel != controller),
            "tooltip render records should remain deferred until first reveal",
        );

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        for _ in 0..TOOLTIP_REVEAL_SETTLE_FRAMES {
            app.update();
            assert_eq!(
                sdf_material_assignments_except(&app, controller),
                peer_materials,
                "first materialization must not renumber an existing SDF material row",
            );
            assert_material_table_buffer_unchanged(
                &app,
                &table_handle,
                table_capacity,
                table_allocations,
            );
        }
        assert_tooltip_surface_is_retained(&app, controller);
    }

    #[test]
    fn tooltip_hide_show_preserves_material_rows_and_table_buffer() {
        let ShownTooltipFixture {
            mut app,
            widget,
            controller,
            ..
        } = shown_tooltip_fixture();
        let visible_material_table = app.world().resource::<FrameMaterialTableBuild>().table();
        let visible_material_rows = visible_material_table.row_count();
        let visible_material_sources = visible_material_table.live_row_count();
        let visible_tooltip_slots = panel_authored_slot_ids(&app, controller);
        assert!(
            !visible_tooltip_slots.is_empty(),
            "a visible tooltip should reference authored material rows",
        );
        let visible_peer_materials = sdf_material_assignments_except(&app, controller);
        let table_buffer = app.world().resource::<MaterialTableBuffer>();
        let table_handle = table_buffer
            .handle
            .clone()
            .expect("the material table should have an allocated buffer");
        let table_capacity = table_buffer.capacity;
        let table_allocations = table_buffer.allocations;
        let batch_gpu_identity = sdf_batch_gpu_identity_for_panel(&app, controller);
        clear_asset_events::<ShaderBuffer>(&mut app);

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::None);
        app.update();
        let hidden_record = sdf_records(&app)
            .into_iter()
            .find(|record| record.record_key.panel == controller)
            .expect("hiding should retain the tooltip's SDF record");
        assert_eq!(hidden_record.paint_mask.bits(), 0);
        assert_eq!(
            sdf_batch_gpu_identity_for_panel(&app, controller),
            batch_gpu_identity,
            "hiding the tooltip must preserve its SDF batch allocation",
        );
        assert_eq!(
            app.world()
                .resource::<FrameMaterialTableBuild>()
                .table()
                .row_count(),
            visible_material_rows,
            "a hidden materialized tooltip should reserve its frame-table rows",
        );
        assert_eq!(
            app.world()
                .resource::<FrameMaterialTableBuild>()
                .table()
                .live_row_count(),
            visible_material_sources,
            "hiding a materialized tooltip should keep its material sources live",
        );
        assert_eq!(
            sdf_material_assignments_except(&app, controller),
            visible_peer_materials,
            "hiding a tooltip should not renumber another SDF record's material rows",
        );
        assert_material_table_buffer_unchanged(
            &app,
            &table_handle,
            table_capacity,
            table_allocations,
        );
        assert_eq!(modified_asset_events_for(&app, &table_handle), 0);
        clear_asset_events::<ShaderBuffer>(&mut app);

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        let reshown_tooltip_slots = panel_authored_slot_ids(&app, controller);
        assert_eq!(
            reshown_tooltip_slots, visible_tooltip_slots,
            "showing the same tooltip again should restore the same material rows",
        );
        let reshown_record = sdf_records(&app)
            .into_iter()
            .find(|record| record.record_key.panel == controller)
            .expect("showing should retain the tooltip's SDF record");
        assert_ne!(reshown_record.paint_mask.bits(), 0);
        assert_eq!(
            sdf_batch_gpu_identity_for_panel(&app, controller),
            batch_gpu_identity,
            "showing the tooltip must preserve its SDF batch allocation",
        );
        assert_eq!(
            sdf_material_assignments_except(&app, controller),
            visible_peer_materials,
            "showing the tooltip again should not renumber another SDF record's material rows",
        );
        assert_material_table_buffer_unchanged(
            &app,
            &table_handle,
            table_capacity,
            table_allocations,
        );
        assert_eq!(modified_asset_events_for(&app, &table_handle), 0);
    }

    #[test]
    fn ready_tooltip_reveal_routes_its_final_transform_in_the_same_frame() {
        let ShownTooltipFixture {
            mut app,
            panel,
            widget,
            controller,
        } = shown_tooltip_fixture();
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::None);
        app.update();
        let mut panel_transform = app
            .world_mut()
            .get_mut::<Transform>(panel)
            .expect("the tooltip target panel should retain its transform");
        panel_transform.translation = Vec3::new(0.4, -0.2, 0.3);
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert_tooltip_retained_transform(&app, controller);

        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, tooltip_surface_tree("Replacement tooltip"))
                .is_ok()
        );
        for _ in 0..TOOLTIP_REVEAL_SETTLE_FRAMES {
            app.update();
        }
        let replacement_controller = app
            .world()
            .get::<Tooltips>(widget)
            .and_then(|tooltips| tooltips.iter().next())
            .expect("replacement tooltip controller should reify");
        assert_ne!(replacement_controller, controller);
        assert!(app.world().get_entity(controller).is_err());
        assert_eq!(
            app.world().get::<Visibility>(replacement_controller),
            Some(&Visibility::Inherited),
        );
        assert_tooltip_surface_is_retained(&app, replacement_controller);
        assert_tooltip_retained_transform(&app, replacement_controller);
    }

    #[test]
    fn secondary_window_tooltip_reveal_routes_its_final_transform() {
        let mut app = sdf_pipeline_app();
        app.add_plugins((WidgetsPlugin, ScreenSpacePlugin));
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        let secondary_window = app
            .world_mut()
            .spawn(Window {
                resolution: UVec2::new(960, 540).into(),
                ..Default::default()
            })
            .id();
        let render_layers = RenderLayers::layer(6);
        let panel = spawn_sdf_screen_panel(
            &mut app,
            tooltip_surface_tree("Retained tooltip"),
            StandardMaterial::default(),
            secondary_window,
            37,
            render_layers.clone(),
        );
        settle_sdf_pipeline(&mut app);
        let widget = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named("tooltip-target"))
            })
            .ok()
            .flatten()
            .expect("screen tooltip target widget should reify");
        let controller = app
            .world()
            .get::<Tooltips>(widget)
            .and_then(|tooltips| tooltips.iter().next())
            .expect("screen tooltip controller should reify");

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        for _ in 0..TOOLTIP_REVEAL_SETTLE_FRAMES {
            app.update();
        }

        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Inherited),
        );
        let panel_layers = app
            .world()
            .get::<RenderLayers>(panel)
            .cloned()
            .expect("screen panel should carry the overlay camera's view layer");
        assert_ne!(
            panel_layers, render_layers,
            "the screen panel renders on its camera's allocated view layer, not the authored one",
        );
        assert_eq!(
            app.world().get::<RenderLayers>(controller),
            Some(&panel_layers),
            "the tooltip should inherit the secondary-window panel's layers",
        );
        let record = sdf_records(&app)
            .into_iter()
            .find(|record| record.record_key.panel == controller)
            .expect("the secondary-window tooltip should publish a retained record");
        let global = app
            .world()
            .get::<GlobalTransform>(controller)
            .expect("revealed screen tooltip should have a propagated transform");
        assert_eq!(
            record.transform,
            global.to_matrix() * record.local_transform.to_matrix(),
            "the retained screen record must contain the final propagated transform",
        );
    }

    fn single_surface_tree(color: Color) -> LayoutTree {
        LayoutBuilder::with_root(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(color),
        )
        .build()
    }

    fn rounded_surface_tree(color: Color) -> LayoutTree {
        LayoutBuilder::with_root(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(color)
                .corner_radius(4.0),
        )
        .build()
    }

    fn stacked_surface_tree(first: Color, second: Color) -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(first),
            |builder| {
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(second),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    fn side_by_side_surface_tree(first: Color, second: Color) -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::row().width(Sizing::GROW).height(Sizing::GROW),
            |builder| {
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(first),
                    |_| {},
                );
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(second),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    // Drives each fill's base color through the production `.background()` path
    // (the layout color override), while the material supplies the animated
    // scalar PBR fields the background does not touch.
    fn two_material_surface_tree(
        first: Handle<StandardMaterial>,
        first_color: Color,
        second: Handle<StandardMaterial>,
        second_color: Color,
    ) -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::column().width(Sizing::GROW).height(Sizing::GROW),
            |builder| {
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(first_color)
                        .material(first),
                    |_| {},
                );
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(second_color)
                        .material(second),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    fn bordered_surface_tree(fill: Color, border: Color) -> LayoutTree {
        LayoutBuilder::with_root(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(fill)
                .border(Border::all(Mm(1.0), border)),
        )
        .build()
    }

    fn border_only_tree() -> LayoutTree {
        LayoutBuilder::with_root(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .border(Border::all(Mm(IMAGE_BORDER_WIDTH_MM), IMAGE_BORDER_COLOR)),
        )
        .build()
    }

    fn clipped_image_border_tree(image: Handle<Image>) -> LayoutTree {
        let mut builder = LayoutBuilder::with_root(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .clip()
                .border(Border::all(Mm(IMAGE_BORDER_WIDTH_MM), IMAGE_BORDER_COLOR)),
        );
        builder.image(
            El::new().width(Sizing::GROW).height(Sizing::GROW),
            image,
            Color::WHITE,
        );
        builder.build()
    }

    fn filled_clipped_image_border_tree(image: Handle<Image>) -> LayoutTree {
        let mut builder = LayoutBuilder::with_root(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .clip()
                .background(Color::BLACK)
                .border(Border::all(Mm(IMAGE_BORDER_WIDTH_MM), IMAGE_BORDER_COLOR)),
        );
        builder.image(
            El::new().width(Sizing::GROW).height(Sizing::GROW),
            image,
            Color::WHITE,
        );
        builder.build()
    }

    fn material_filled_clipped_image_border_tree(
        image: Handle<Image>,
        material: Handle<StandardMaterial>,
    ) -> LayoutTree {
        let mut builder = LayoutBuilder::with_root(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .clip()
                .background(Color::BLACK)
                .material(material)
                .border(Border::all(Mm(IMAGE_BORDER_WIDTH_MM), IMAGE_BORDER_COLOR)),
        );
        builder.image(
            El::new().width(Sizing::GROW).height(Sizing::GROW),
            image,
            Color::WHITE,
        );
        builder.build()
    }

    /// Text-command state used by `text_toggle_tree`.
    #[derive(Clone, Copy)]
    enum TextContentState {
        /// The tree includes one text command after the background.
        Present,
        /// The tree includes only the background command.
        Removed,
    }

    fn text_toggle_tree(text_content_state: TextContentState) -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(Color::WHITE),
            |builder| {
                if matches!(text_content_state, TextContentState::Present) {
                    builder.text(("Alpha", TextStyle::new(10.0)));
                }
            },
        );
        builder.build()
    }

    fn topology_churn_tree(survivor: Color, added: Color) -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::new().width(Sizing::GROW).height(Sizing::GROW),
            |builder| {
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(survivor),
                    |_| {},
                );
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(added),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    fn sdf_records(app: &App) -> Vec<ResolvedSdfBatchRecord> {
        let mut records: Vec<ResolvedSdfBatchRecord> = app
            .world()
            .resource::<SdfBatchStore>()
            .batches()
            .flat_map(|(_, batch)| batch.records().iter().cloned())
            .collect();
        records.sort_by_key(|record| record.draw_order);
        records
    }

    fn image_records(app: &App) -> Vec<ResolvedImageRecord> {
        let mut records: Vec<ResolvedImageRecord> = app
            .world()
            .resource::<ImageBatchStore>()
            .batches()
            .flat_map(|(_, batch)| batch.records().iter().cloned())
            .collect();
        records.sort_by(|left, right| {
            left.record_key
                .command_index
                .cmp(&right.record_key.command_index)
        });
        records
    }

    fn single_sdf_batch_records(app: &App) -> Vec<ResolvedSdfBatchRecord> {
        let store = app.world().resource::<SdfBatchStore>();
        let batches: Vec<&SdfBatch> = store.batches().map(|(_, batch)| batch).collect();
        assert_eq!(batches.len(), 1, "expected exactly one SDF batch");
        batches[0].records().to_vec()
    }

    fn authored_fill_draw_orders(app: &App) -> Vec<CommandIndex> {
        single_sdf_batch_records(app)
            .into_iter()
            .filter(|record| matches!(record.fill_material, SdfPaintMaterial::Authored(_)))
            .map(|record| record.draw_order)
            .collect()
    }

    fn reset_sdf_batch_sort_counts(app: &mut App) {
        for (_, batch) in app
            .world_mut()
            .resource_mut::<SdfBatchStore>()
            .batches_mut()
        {
            batch.sort_count = 0;
        }
    }

    fn sdf_batch_sort_counts(app: &App) -> Vec<usize> {
        app.world()
            .resource::<SdfBatchStore>()
            .batches()
            .map(|(_, batch)| batch.sort_count)
            .collect()
    }

    fn live_sdf_batch_count(app: &mut App) -> usize {
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<DiegeticSdfFillBatch>>();
        query.iter(app.world()).count()
    }

    fn live_sdf_batch_entities(app: &mut App) -> Vec<Entity> {
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<DiegeticSdfFillBatch>>();
        let mut entities: Vec<Entity> = query.iter(app.world()).collect();
        entities.sort_by_key(|entity| entity.to_bits());
        entities
    }

    fn sdf_batch_visibilities(app: &mut App) -> Vec<Visibility> {
        let mut query = app
            .world_mut()
            .query_filtered::<&Visibility, With<DiegeticSdfFillBatch>>();
        query.iter(app.world()).copied().collect()
    }

    fn linear_color(color: Color) -> Vec4 {
        let linear = color.to_linear();
        Vec4::new(linear.red, linear.green, linear.blue, linear.alpha)
    }

    fn fill_row_color(app: &App, record: &ResolvedSdfBatchRecord) -> Vec4 {
        authored_row_color(app, record.fill_material)
    }

    fn border_row_color(app: &App, record: &ResolvedSdfBatchRecord) -> Vec4 {
        authored_row_color(app, record.border_material)
    }

    fn authored_row(app: &App, material: SdfPaintMaterial) -> MaterialSlotValues {
        let SdfPaintMaterial::Authored(slot) = material else {
            panic!("material role should be authored");
        };
        let row_index = usize::try_from(slot.as_u32()).expect("slot index should fit usize");
        app.world()
            .resource::<FrameMaterialTableBuild>()
            .table()
            .rows()[row_index]
    }

    fn authored_row_color(app: &App, material: SdfPaintMaterial) -> Vec4 {
        authored_row(app, material).base_color
    }

    fn fill_row_metallic(app: &App, record: &ResolvedSdfBatchRecord) -> f32 {
        authored_row(app, record.fill_material).metallic
    }

    fn assert_same_f32(actual: f32, expected: f32) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }

    fn frame_material_row_count(app: &App) -> usize {
        app.world()
            .resource::<FrameMaterialTableBuild>()
            .table()
            .row_count()
    }

    fn frame_material_live_row_count(app: &App) -> usize {
        app.world()
            .resource::<FrameMaterialTableBuild>()
            .table()
            .live_row_count()
    }

    fn frame_material_capacity(app: &App) -> u32 {
        app.world().resource::<MaterialTableBuffer>().capacity
    }

    fn authored_slot_ids(records: &[ResolvedSdfBatchRecord]) -> Vec<MaterialSlotId> {
        records
            .iter()
            .flat_map(|record| [record.fill_material, record.border_material])
            .filter_map(|material| match material {
                SdfPaintMaterial::Authored(slot) => Some(slot),
                SdfPaintMaterial::NotAuthored => None,
            })
            .collect()
    }

    fn panel_authored_slot_ids(app: &App, panel: Entity) -> Vec<MaterialSlotId> {
        let records = sdf_records(app)
            .into_iter()
            .filter(|record| record.record_key.panel == panel)
            .collect::<Vec<_>>();
        authored_slot_ids(&records)
    }

    fn sdf_material_assignments_except(
        app: &App,
        excluded_panel: Entity,
    ) -> Vec<(
        CommandIndex,
        SdfRecordKey,
        SdfPaintMaterial,
        SdfPaintMaterial,
    )> {
        let mut assignments: Vec<_> = sdf_records(app)
            .into_iter()
            .filter(|record| record.record_key.panel != excluded_panel)
            .map(|record| {
                (
                    record.draw_order,
                    record.record_key,
                    record.fill_material,
                    record.border_material,
                )
            })
            .collect();
        assignments.sort_by_key(|(draw_order, record_key, _, _)| {
            (record_key.panel.to_bits(), draw_order.get())
        });
        assignments
    }

    fn assert_material_table_buffer_unchanged(
        app: &App,
        expected_handle: &Handle<ShaderBuffer>,
        expected_capacity: u32,
        expected_allocations: LifetimeTotal,
    ) {
        let table_buffer = app.world().resource::<MaterialTableBuffer>();
        assert_eq!(table_buffer.handle.as_ref(), Some(expected_handle));
        assert_eq!(table_buffer.capacity, expected_capacity);
        assert_eq!(table_buffer.allocations, expected_allocations);
    }

    fn authored_slot_count(records: &[ResolvedSdfBatchRecord]) -> usize {
        authored_slot_ids(records).len()
    }

    fn assert_authored_slots_are_distinct(records: &[ResolvedSdfBatchRecord]) {
        let slot_ids = authored_slot_ids(records);
        let distinct_slot_ids: HashSet<_> = slot_ids.iter().copied().collect();
        assert_eq!(distinct_slot_ids.len(), slot_ids.len());
    }

    fn assert_fill_row_colors(
        app: &App,
        records: &[ResolvedSdfBatchRecord],
        expected_colors: &[Color],
    ) {
        assert_eq!(records.len(), expected_colors.len());
        for (record, expected_color) in records.iter().zip(expected_colors) {
            assert_eq!(fill_row_color(app, record), linear_color(*expected_color));
        }
    }

    fn animated_scalar_material(frame: usize, base: Color, metallic: f32) -> StandardMaterial {
        let frame = frame.to_f32();
        let base = base.to_srgba();
        StandardMaterial {
            base_color: Color::srgb(
                (frame * ANIMATED_BASE_RED_SPEED)
                    .sin()
                    .mul_add(ANIMATED_BASE_COLOR_SWING, base.red)
                    .clamp(0.0, 1.0),
                (frame * ANIMATED_BASE_GREEN_SPEED)
                    .sin()
                    .mul_add(ANIMATED_BASE_COLOR_SWING, base.green)
                    .clamp(0.0, 1.0),
                (frame * ANIMATED_BASE_BLUE_SPEED)
                    .sin()
                    .mul_add(ANIMATED_BASE_COLOR_SWING, base.blue)
                    .clamp(0.0, 1.0),
            ),
            metallic,
            perceptual_roughness: frame.mul_add(0.03, 0.25).min(0.9),
            reflectance: frame.mul_add(0.02, 0.35).min(0.9),
            ..Default::default()
        }
    }

    fn clear_asset_events<A: Asset>(app: &mut App) {
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<A>>>()
            .clear();
    }

    fn modified_asset_events<A: Asset>(app: &App) -> usize {
        app.world()
            .resource::<Messages<AssetEvent<A>>>()
            .iter_current_update_messages()
            .filter(|event| matches!(event, AssetEvent::Modified { .. }))
            .count()
    }

    fn modified_asset_events_for<A: Asset>(app: &App, handle: &Handle<A>) -> usize {
        let id = handle.id();
        app.world()
            .resource::<Messages<AssetEvent<A>>>()
            .iter_current_update_messages()
            .filter(
                |event| matches!(event, AssetEvent::Modified { id: modified } if *modified == id),
            )
            .count()
    }

    fn crate_wgsl_files() -> Vec<PathBuf> {
        let mut files = Vec::new();
        collect_wgsl_files(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        files.sort();
        files
    }

    fn collect_wgsl_files(path: &Path, files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).expect("crate source directory should be readable") {
            let entry = entry.expect("crate source entry should be readable");
            let path = entry.path();
            if path.is_dir() {
                collect_wgsl_files(&path, files);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "wgsl")
            {
                files.push(path);
            }
        }
    }

    fn function_body_range(source: &str, signature: &str) -> Range<usize> {
        let signature_start = source
            .find(signature)
            .expect("function signature should exist");
        let body_start = signature_start
            + source[signature_start..]
                .find('{')
                .expect("function body should start");
        let mut depth = 0_u32;
        for (offset, byte) in source[body_start..].bytes().enumerate() {
            match byte {
                b'{' => depth = depth.saturating_add(1),
                b'}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return body_start..body_start + offset + 1;
                    }
                },
                _ => {},
            }
        }
        panic!("function body should close");
    }

    fn storage_binding_for_field(source: &str, field_name: &str) -> u32 {
        let struct_body = &source[function_body_range(source, "pub(crate) struct SdfExtension")];
        let field_offset = struct_body
            .find(&format!("{field_name}:"))
            .unwrap_or_else(|| panic!("{field_name} should be an SdfExtension field"));
        let attr_prefix = "#[storage(";
        let attr_offset = struct_body[..field_offset]
            .rfind(attr_prefix)
            .unwrap_or_else(|| panic!("{field_name} should have a storage binding"));
        let binding_start = attr_offset + attr_prefix.len();
        let binding_digits: String = struct_body[binding_start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        binding_digits
            .parse()
            .expect("storage binding should be a u32 literal")
    }

    fn assert_wgsl_storage_binding(source: &str, binding: u32, field_name: &str) {
        assert!(
            source.contains(&format!(
                "@binding({binding}) var<storage, read> {field_name}"
            )),
            "{field_name} should use binding {binding}"
        );
    }

    fn assert_wgsl_binding_constant(source: &str, binding_name: &str, binding: u32) {
        assert!(
            source.contains(&format!("const {binding_name}: u32 = {binding}u;")),
            "{binding_name} should equal {binding}"
        );
    }

    fn assert_sdf_shader_ref(shader_ref: ShaderRef) {
        let ShaderRef::Path(path) = shader_ref else {
            panic!("SdfExtension shader entry point should use the shared SDF shader path");
        };
        assert_eq!(path.to_string(), EMBEDDED_SDF_PANEL_BATCH_SHADER_PATH);
    }

    fn assert_ordered(source: &str, ordered_needles: &[&str]) {
        let mut previous = 0;
        for needle in ordered_needles {
            let offset = source[previous..]
                .find(needle)
                .unwrap_or_else(|| panic!("{needle} should appear after byte {previous}"));
            previous += offset + needle.len();
        }
    }

    fn fill_input(material: &StandardMaterial, color: Option<Color>) -> SdfMaterialSlotInput<'_> {
        SdfMaterialSlotInput {
            key:            SdfMaterialSourceKey {
                panel:           Entity::from_bits(1),
                element:         ElementIndex::from(0),
                divider_ordinal: None,
                role:            SdfMaterialRole::Fill,
            },
            base_material:  material,
            color_override: color,
            lighting:       Lighting::Lit,
            sidedness:      Sidedness::BothSides,
        }
    }

    #[test]
    fn material_slot_input_projects_color_override_into_table_values() {
        let material = StandardMaterial {
            base_color: Color::srgb(0.1, 0.2, 0.3),
            ..Default::default()
        };
        let input = fill_input(&material, Some(Color::srgba(0.8, 0.7, 0.6, 0.5)));
        let candidate = input.material_slot_candidate();
        let expected = Color::srgba(0.8, 0.7, 0.6, 0.5).to_linear();

        assert_eq!(
            candidate.values.base_color,
            Vec4::new(expected.red, expected.green, expected.blue, expected.alpha)
        );
        assert_eq!(candidate.pipeline_compatibility, {
            let mut expected_material = StandardMaterial {
                base_color: Color::srgba(0.8, 0.7, 0.6, 0.5),
                ..material
            };
            render::apply_sidedness(&mut expected_material, Sidedness::BothSides);
            PipelineCompatibility::from(&expected_material)
        });
    }

    #[test]
    fn sdf_material_slot_input_uses_cascade_lighting_and_sidedness() {
        let mut base_material = StandardMaterial {
            unlit: true,
            ..Default::default()
        };
        render::apply_sidedness(&mut base_material, Sidedness::BackOnly);
        let input = SdfMaterialSlotInput {
            key:            SdfMaterialSourceKey {
                panel:           Entity::from_bits(1),
                element:         ElementIndex::from(0),
                divider_ordinal: None,
                role:            SdfMaterialRole::Fill,
            },
            base_material:  &base_material,
            color_override: None,
            lighting:       Lighting::Lit,
            sidedness:      Sidedness::BothSides,
        };
        let mut expected_material = base_material.clone();
        render::apply_sidedness(&mut expected_material, Sidedness::BothSides);
        expected_material.unlit = false;
        let expected_candidate = MaterialSlotCandidate::from(&expected_material);

        let candidate = input.material_slot_candidate();

        assert_eq!(candidate.values, expected_candidate.values);
        assert_eq!(
            candidate.pipeline_compatibility,
            expected_candidate.pipeline_compatibility
        );
        assert_ne!(
            candidate.pipeline_compatibility,
            PipelineCompatibility::from(&base_material)
        );
    }

    #[test]
    fn paint_mask_and_gpu_slots_distinguish_fill_border_and_absent_roles() {
        let fill = SdfPaintMaterial::Authored(MaterialSlotId::try_from(0).expect("slot 0 is live"));
        let border = SdfPaintMaterial::NotAuthored;
        let mask = SdfPaintMask::from_materials(fill, border);

        assert_eq!(mask.bits(), SdfPaintMask::FILL);
        assert_eq!(fill.to_gpu().as_u32(), 0);
        assert_eq!(border.to_gpu().as_u32(), INVALID_GPU_MATERIAL_SLOT);
        assert_eq!(SdfRenderRecord::padded().paint_mask.bits(), 0);
        assert_eq!(
            SdfRenderRecord::padded().fill_material.as_u32(),
            INVALID_GPU_MATERIAL_SLOT
        );
    }

    #[test]
    fn first_role_limit_drop_skips_the_whole_record() {
        let mut builder = FrameMaterialTableBuilder::default();
        builder.clear(0);
        let (materials, default_material, material) =
            material_context_for_test(StandardMaterial::default());
        let surface = resolved_surface_for_test(
            0,
            &material,
            SdfRoleAuthorship::Authored,
            SdfRoleAuthorship::Authored,
        );
        let asset_server = asset_server_for_test();

        let append = append_sdf_record_materials(
            &mut builder,
            &surface,
            Lighting::Lit,
            Sidedness::BothSides,
            &materials,
            &asset_server,
            &default_material,
        );

        assert_eq!(append, SdfRecordRouting::SlotLimit);
        assert_eq!(builder.row_count(), 0);
        assert_eq!(builder.dropped_record_count(), 1);
    }

    #[test]
    fn post_fill_border_limit_drop_rolls_back_the_record() {
        let mut builder = FrameMaterialTableBuilder::default();
        builder.clear(1);
        let (materials, default_material, material) =
            material_context_for_test(StandardMaterial::default());
        let surface = resolved_surface_for_test(
            0,
            &material,
            SdfRoleAuthorship::Authored,
            SdfRoleAuthorship::Authored,
        );
        let asset_server = asset_server_for_test();

        let append = append_sdf_record_materials(
            &mut builder,
            &surface,
            Lighting::Lit,
            Sidedness::BothSides,
            &materials,
            &asset_server,
            &default_material,
        );

        assert_eq!(append, SdfRecordRouting::SlotLimit);
        assert_eq!(builder.row_count(), 0);
        assert_eq!(builder.dropped_record_count(), 1);
    }

    #[test]
    fn border_limit_drop_counts_each_rolled_back_surface() {
        let mut builder = FrameMaterialTableBuilder::default();
        builder.clear(1);
        let (materials, default_material, material) =
            material_context_for_test(StandardMaterial::default());
        let first_surface = resolved_surface_for_test(
            0,
            &material,
            SdfRoleAuthorship::Authored,
            SdfRoleAuthorship::Authored,
        );
        let second_surface = resolved_surface_for_test(
            1,
            &material,
            SdfRoleAuthorship::Authored,
            SdfRoleAuthorship::Authored,
        );
        let asset_server = asset_server_for_test();

        let first = append_sdf_record_materials(
            &mut builder,
            &first_surface,
            Lighting::Lit,
            Sidedness::BothSides,
            &materials,
            &asset_server,
            &default_material,
        );
        let second = append_sdf_record_materials(
            &mut builder,
            &second_surface,
            Lighting::Lit,
            Sidedness::BothSides,
            &materials,
            &asset_server,
            &default_material,
        );

        assert_eq!(first, SdfRecordRouting::SlotLimit);
        assert_eq!(second, SdfRecordRouting::SlotLimit);
        assert_eq!(builder.row_count(), 0);
        assert_eq!(builder.dropped_record_count(), 2);
    }

    #[test]
    fn two_role_append_keeps_fill_and_border_when_both_rows_fit() {
        let mut builder = FrameMaterialTableBuilder::default();
        let (materials, default_material, material) =
            material_context_for_test(StandardMaterial::default());
        let surface = resolved_surface_for_test(
            0,
            &material,
            SdfRoleAuthorship::Authored,
            SdfRoleAuthorship::Authored,
        );
        builder.clear(2);
        let asset_server = asset_server_for_test();
        let append = append_sdf_record_materials(
            &mut builder,
            &surface,
            Lighting::Lit,
            Sidedness::BothSides,
            &materials,
            &asset_server,
            &default_material,
        );
        let SdfRecordRouting::Routed(append) = append else {
            panic!("both authored roles fit when two rows remain");
        };

        assert_eq!(builder.row_count(), 2);
        assert!(matches!(append.fill, SdfPaintMaterial::Authored(_)));
        assert!(matches!(append.border, SdfPaintMaterial::Authored(_)));
    }

    #[test]
    fn sdf_fallback_material_uses_blend_for_roles_without_base_material() {
        let mut builder = FrameMaterialTableBuilder::default();
        let material = StandardMaterial {
            alpha_mode: AlphaMode::Opaque,
            ..Default::default()
        };
        let (materials, default_material, material) = material_context_for_test(material);
        let mut surface = resolved_surface_for_test(
            0,
            &material,
            SdfRoleAuthorship::Authored,
            SdfRoleAuthorship::Authored,
        );
        surface.fill_material.base_material = None;
        surface.border_material.base_material = None;
        builder.clear(2);
        let asset_server = asset_server_for_test();

        let append = append_sdf_record_materials(
            &mut builder,
            &surface,
            Lighting::Lit,
            Sidedness::BothSides,
            &materials,
            &asset_server,
            &default_material,
        );
        let SdfRecordRouting::Routed(append) = append else {
            panic!("color-authored SDF roles should append fallback material rows");
        };

        assert_eq!(builder.row_count(), 2);
        assert_eq!(append.pipeline_compatibility.alpha, BatchAlphaMode::Blend);
        assert!(matches!(append.fill, SdfPaintMaterial::Authored(_)));
        assert!(matches!(append.border, SdfPaintMaterial::Authored(_)));
    }

    #[test]
    fn slot_zero_can_be_live_while_padded_records_are_absent() {
        let mut builder = FrameMaterialTableBuilder::default();
        builder.clear(4);
        let material = StandardMaterial::default();
        let input = fill_input(&material, Some(Color::WHITE));

        let append = material_table::append_material_slot(&mut builder, &input);
        let MaterialSlotAppend::Appended(appended) = append else {
            panic!("first slot should append");
        };

        assert_eq!(
            FrameMaterialSlotAppend::Appended(appended.slot),
            FrameMaterialSlotAppend::Appended(MaterialSlotId::try_from(0).expect("slot 0"))
        );
        assert_eq!(
            SdfPaintMaterial::Authored(appended.slot).to_gpu().as_u32(),
            0
        );
        assert_eq!(
            SdfRenderRecord::padded().border_material.as_u32(),
            INVALID_GPU_MATERIAL_SLOT
        );
    }

    #[test]
    fn scalar_pbr_edits_do_not_change_sdf_batch_key_compatibility() {
        let mut first = StandardMaterial {
            base_color: Color::srgb(0.1, 0.2, 0.3),
            metallic: 0.1,
            perceptual_roughness: 0.2,
            reflectance: 0.3,
            ..Default::default()
        };
        let second = StandardMaterial {
            base_color: Color::srgb(0.8, 0.7, 0.6),
            metallic: 0.9,
            perceptual_roughness: 0.8,
            reflectance: 0.7,
            ..Default::default()
        };
        first.alpha_mode = AlphaMode::Blend;

        assert_eq!(
            PipelineCompatibility::from(&first),
            PipelineCompatibility::from(&StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                ..second.clone()
            })
        );
        assert_eq!(
            ResourceCompatibility::from(&first),
            ResourceCompatibility::from(&second)
        );
    }

    #[test]
    fn alpha_mode_and_culling_configure_sdf_batch_material() {
        for shadow in [VisualShadow::None, VisualShadow::Cast] {
            for cull_mode in [None, Some(Face::Back), Some(Face::Front)] {
                let mut pipeline_compatibility = PipelineCompatibility::from(&StandardMaterial {
                    alpha_mode: AlphaMode::Opaque,
                    double_sided: cull_mode.is_none(),
                    cull_mode,
                    unlit: true,
                    fog_enabled: false,
                    deferred_lighting_pass_id: 3,
                    ..Default::default()
                });
                pipeline_compatibility.double_sided = cull_mode.is_none();
                let key = SdfBatchKey {
                    z_index: 0.into(),
                    z_index_rank: DrawZIndexRank::default(),
                    batch_family: DrawBatchFamily::SdfSurface,
                    layers: BatchRenderLayers(RenderLayers::layer(0)),
                    shadow,
                    contiguous_drawn_run: ContiguousDrawnRun::default(),
                    pipeline_compatibility,
                    resource_compatibility: ResourceCompatibility::from(
                        &StandardMaterial::default(),
                    ),
                };
                let material = sdf_batch_material(SdfBatchMaterialInput {
                    key,
                    records: Handle::default(),
                    mesh_records: Handle::default(),
                });

                let expected_alpha = match shadow {
                    VisualShadow::Cast => AlphaMode::Mask(0.0),
                    VisualShadow::None => AlphaMode::Opaque,
                };
                assert_eq!(material.base.alpha_mode, expected_alpha);
                assert_eq!(material.base.cull_mode, cull_mode);
                assert_eq!(material.base.double_sided, cull_mode.is_none());
                assert!(material.base.unlit);
                assert!(!material.base.fog_enabled);
                assert_eq!(material.base.deferred_lighting_pass_id, 3);
            }
        }
    }

    #[test]
    fn opaque_sdf_batches_use_mask_gpu_alpha_mode() {
        // Casting opaque batches map to Mask(0.0) to retain the material group on
        // the shadow/prepass pipeline; non-casting opaque batches stay Opaque.
        assert_eq!(
            sdf_batch_alpha_mode(AlphaMode::Opaque.into(), VisualShadow::Cast),
            AlphaMode::Mask(0.0)
        );
        assert_eq!(
            sdf_batch_alpha_mode(AlphaMode::Opaque.into(), VisualShadow::None),
            AlphaMode::Opaque
        );
        for mode in [
            AlphaMode::Mask(0.25),
            AlphaMode::Blend,
            AlphaMode::Premultiplied,
            AlphaMode::Add,
            AlphaMode::Multiply,
            AlphaMode::AlphaToCoverage,
        ] {
            for shadow in [VisualShadow::None, VisualShadow::Cast] {
                assert_eq!(sdf_batch_alpha_mode(mode.into(), shadow), mode);
            }
        }
    }

    #[test]
    fn interleaved_compatibility_keys_split_into_maximal_draw_runs() {
        let material_a = StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            ..Default::default()
        };
        let material_b = StandardMaterial {
            alpha_mode: AlphaMode::Add,
            ..Default::default()
        };
        let mut materials = Assets::<StandardMaterial>::default();
        let handle_a = materials.add(material_a.clone());
        let handle_b = materials.add(material_b.clone());
        let surface_0 = resolved_surface_for_test(
            0,
            &handle_a,
            SdfRoleAuthorship::Authored,
            SdfRoleAuthorship::Unauthored,
        );
        let mut surface_1 = resolved_surface_for_test(
            1,
            &handle_a,
            SdfRoleAuthorship::Authored,
            SdfRoleAuthorship::Unauthored,
        );
        surface_1.panel_entity = Entity::from_bits(2);
        let surface_2 = resolved_surface_for_test(
            2,
            &handle_b,
            SdfRoleAuthorship::Authored,
            SdfRoleAuthorship::Unauthored,
        );
        let surface_3 = resolved_surface_for_test(
            3,
            &handle_a,
            SdfRoleAuthorship::Authored,
            SdfRoleAuthorship::Unauthored,
        );
        let records = assign_contiguous_runs(vec![
            (slots_for_test(0, &material_a), &surface_0),
            (slots_for_test(1, &material_a), &surface_1),
            (slots_for_test(2, &material_b), &surface_2),
            (slots_for_test(3, &material_a), &surface_3),
        ]);

        let runs: Vec<u32> = records
            .iter()
            .map(|record| record.batch_key.contiguous_drawn_run.value)
            .collect();

        assert_eq!(runs, vec![0, 0, 1, 2]);
        assert_eq!(records[0].batch_key, records[1].batch_key);
        assert_ne!(records[1].batch_key, records[2].batch_key);
        assert_ne!(records[2].batch_key, records[3].batch_key);
    }

    #[test]
    fn interleaved_render_layers_assign_draw_runs_independently() {
        let material = StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            ..Default::default()
        };
        let mut materials = Assets::<StandardMaterial>::default();
        let handle = materials.add(material.clone());
        let mut surface_0 = resolved_surface_for_test(
            0,
            &handle,
            SdfRoleAuthorship::Authored,
            SdfRoleAuthorship::Unauthored,
        );
        surface_0.render_layers = RenderLayers::layer(0);
        let mut surface_1 = resolved_surface_for_test(
            1,
            &handle,
            SdfRoleAuthorship::Authored,
            SdfRoleAuthorship::Unauthored,
        );
        surface_1.render_layers = RenderLayers::layer(1);
        let mut surface_2 = resolved_surface_for_test(
            2,
            &handle,
            SdfRoleAuthorship::Authored,
            SdfRoleAuthorship::Unauthored,
        );
        surface_2.panel_entity = Entity::from_bits(2);
        surface_2.render_layers = RenderLayers::layer(0);

        let records = assign_contiguous_runs(vec![
            (slots_for_test(0, &material), &surface_0),
            (slots_for_test(1, &material), &surface_1),
            (slots_for_test(2, &material), &surface_2),
        ]);

        let layer_zero = BatchRenderLayers(RenderLayers::layer(0));
        let layer_zero_records: Vec<&ResolvedSdfBatchRecord> = records
            .iter()
            .filter(|record| record.batch_key.layers == layer_zero)
            .collect();
        let batch_keys: HashSet<&SdfBatchKey> =
            records.iter().map(|record| &record.batch_key).collect();

        assert_eq!(records.len(), 3);
        assert_eq!(batch_keys.len(), 2);
        assert_eq!(layer_zero_records.len(), 2);
        assert_eq!(
            layer_zero_records[0].batch_key.contiguous_drawn_run,
            layer_zero_records[1].batch_key.contiguous_drawn_run
        );
        assert_eq!(
            layer_zero_records[0].batch_key,
            layer_zero_records[1].batch_key
        );
    }

    #[test]
    fn compatible_sdf_surfaces_from_separate_panels_share_one_batch() {
        let mut app = sdf_pipeline_app();
        let first_panel = spawn_sdf_panel(
            &mut app,
            single_surface_tree(Color::srgb(0.1, 0.2, 0.3)),
            StandardMaterial::default(),
        );
        let second_panel = spawn_sdf_panel(
            &mut app,
            single_surface_tree(Color::srgb(0.6, 0.5, 0.4)),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let records = single_sdf_batch_records(&app);
        let panels: HashSet<Entity> = records
            .iter()
            .map(|record| record.record_key.panel)
            .collect();

        assert_eq!(live_sdf_batch_count(&mut app), 1);
        assert_eq!(records.len(), 2);
        assert_eq!(panels.len(), 2);
        assert!(panels.contains(&first_panel));
        assert!(panels.contains(&second_panel));
    }

    #[test]
    fn batch_bounds_use_clipped_world_corners() {
        let mut batch = SdfBatch::default();
        let record = ResolvedSdfBatchRecord {
            record_key:       SdfRecordKey {
                panel:           Entity::from_bits(1),
                element:         ElementIndex::from(0),
                divider_ordinal: None,
                role:            SdfRecordRole::Whole,
            },
            draw_order:       CommandIndex::from(0),
            fill_source:      SdfMaterialSourceKey {
                panel:           Entity::from_bits(1),
                element:         ElementIndex::from(0),
                divider_ordinal: None,
                role:            SdfMaterialRole::Fill,
            },
            border_source:    SdfMaterialSourceKey {
                panel:           Entity::from_bits(1),
                element:         ElementIndex::from(0),
                divider_ordinal: None,
                role:            SdfMaterialRole::Border,
            },
            draw_depth:       draw_depth_for_test(0),
            batch_key:        test_batch_key(),
            local_transform:  Transform::IDENTITY,
            transform:        Mat4::from_translation(Vec3::new(2.0, 3.0, 4.0)),
            half_size:        SdfHalfSize { value: Vec2::ONE },
            mesh_half_size:   MeshHalfSize {
                value: Vec2::splat(2.0),
            },
            corner_radii:     CornerRadii { value: Vec4::ZERO },
            border_widths:    BorderWidths { value: Vec4::ZERO },
            clip_rect:        LocalClipRect {
                value: Vec4::new(-1.0, -2.0, 2.0, 1.0),
            },
            fill_material:    SdfPaintMaterial::NotAuthored,
            border_material:  SdfPaintMaterial::NotAuthored,
            paint_mask:       SdfPaintMask::empty(),
            clip_depth_nudge: 0.0,
            oit_depth_offset: 0.0,
            flags:            0,
        };
        batch.upsert_record(record);

        let (min, max) = batch.world_bounds().expect("clipped record has bounds");

        assert_eq!(min, Vec3::new(1.0, 1.0, 4.0));
        assert_eq!(max, Vec3::new(4.0, 4.0, 4.0));
    }

    #[test]
    fn fully_clipped_records_do_not_contribute_bounds() {
        assert_eq!(
            clipped_local_bounds(
                MeshHalfSize { value: Vec2::ONE },
                LocalClipRect {
                    value: Vec4::new(2.0, 2.0, 3.0, 3.0),
                },
            ),
            None
        );
    }

    #[test]
    fn sdf_batch_systems_run_in_driver_order() {
        let mut app = sdf_pipeline_app();
        app.init_resource::<SdfDriverRunOrder>();
        spawn_sdf_panel(
            &mut app,
            single_surface_tree(Color::WHITE),
            StandardMaterial::default(),
        );

        app.update();

        let run_order = app.world().resource::<SdfDriverRunOrder>();
        assert_eq!(run_order.names, material_table::SDF_DRIVER_RUN_ORDER);
        assert!(
            run_order.registered_batch_entity,
            "register_sdf_batch_materials should see the batch entity spawned by reconcile_sdf_batch_entities"
        );
    }

    #[test]
    fn sdf_batch_entities_spawn_visible_for_production_route() {
        let mut app = sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            single_surface_tree(Color::WHITE),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let visibilities = sdf_batch_visibilities(&mut app);

        assert_eq!(visibilities.len(), 1);
        assert!(
            visibilities
                .iter()
                .all(|visibility| *visibility != Visibility::Hidden),
            "production SDF batch entities must not spawn hidden"
        );
    }

    #[test]
    fn sdf_batch_entities_ignore_stock_mesh_picking() {
        let mut app = sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            single_surface_tree(Color::WHITE),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let entities = live_sdf_batch_entities(&mut app);
        assert_eq!(entities.len(), 1);
        let pickable = app
            .world()
            .get::<Pickable>(entities[0])
            .expect("SDF batch entity should carry Pickable::IGNORE");
        assert!(!pickable.is_hoverable);
        assert!(!pickable.should_block_lower);
    }

    #[test]
    fn unchanged_sdf_frame_modifies_no_main_world_render_assets() {
        let mut app = sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            single_surface_tree(Color::srgba(0.2, 0.4, 0.6, 0.5)),
            StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                ..Default::default()
            },
        );
        settle_sdf_pipeline(&mut app);
        clear_asset_events::<SdfExtendedMaterial>(&mut app);
        clear_asset_events::<PathExtendedMaterial>(&mut app);
        clear_asset_events::<ShaderBuffer>(&mut app);

        app.update();

        assert_eq!(modified_asset_events::<SdfExtendedMaterial>(&app), 0);
        assert_eq!(modified_asset_events::<PathExtendedMaterial>(&app), 0);
        assert_eq!(modified_asset_events::<ShaderBuffer>(&app), 0);
    }

    #[test]
    fn same_capacity_sdf_edit_stages_gpu_update_in_the_same_frame() {
        let mut app = sdf_pipeline_app();
        let panel = spawn_sdf_panel(
            &mut app,
            single_surface_tree(Color::WHITE),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        let identity = sdf_batch_gpu_identity_for_panel(&app, panel);
        app.update();
        assert!(
            !app.world()
                .resource::<RetainedBatchBufferUploads>()
                .contains(&identity.records),
            "an unchanged frame should leave no staged SDF upload",
        );

        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, rounded_surface_tree(Color::WHITE))
                .is_ok()
        );
        app.update();

        let after = sdf_batch_gpu_identity_for_panel(&app, panel);
        assert_eq!(after.records, identity.records);
        assert!(
            app.world()
                .resource::<RetainedBatchBufferUploads>()
                .contains(&identity.records),
            "the retained SDF edit should stage a direct same-frame GPU write",
        );
    }

    #[test]
    fn text_toggle_keeps_positive_sdf_oit_depth_without_replacing_batch_entity() {
        let mut app = sdf_pipeline_app();
        let panel = spawn_sdf_panel(
            &mut app,
            text_toggle_tree(TextContentState::Present),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let entity_before = live_sdf_batch_entities(&mut app);
        let records_before = sdf_records(&app);
        assert_eq!(entity_before.len(), 1);
        assert_eq!(records_before.len(), 1);
        assert_eq!(
            records_before[0].oit_depth_offset.to_bits(),
            crate::render::constants::OIT_DEPTH_STEP.to_bits()
        );

        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, text_toggle_tree(TextContentState::Removed))
                .is_ok()
        );
        settle_sdf_pipeline(&mut app);

        let entity_after = live_sdf_batch_entities(&mut app);
        let records_after = sdf_records(&app);
        assert_eq!(entity_after, entity_before);
        assert_eq!(records_after.len(), 1);
        assert_eq!(
            records_after[0].oit_depth_offset.to_bits(),
            crate::render::constants::OIT_DEPTH_STEP.to_bits()
        );
    }

    #[test]
    fn opaque_sdf_fill_is_pushed_back_relative_to_a_blend_fill() {
        let fill_clip_depth_nudge = |alpha_mode: AlphaMode| {
            let mut app = sdf_pipeline_app();
            spawn_sdf_panel(
                &mut app,
                single_surface_tree(Color::WHITE),
                StandardMaterial {
                    alpha_mode,
                    ..default()
                },
            );
            settle_sdf_pipeline(&mut app);
            let records = sdf_records(&app);
            assert_eq!(records.len(), 1);
            records[0].clip_depth_nudge
        };

        let opaque_nudge = fill_clip_depth_nudge(AlphaMode::Opaque);
        let blend_nudge = fill_clip_depth_nudge(AlphaMode::Blend);

        assert_eq!(
            opaque_nudge.to_bits(),
            (blend_nudge - OPAQUE_FILL_DEPTH_PUSH_LAYERS).to_bits()
        );
    }

    #[test]
    fn transparent_clipped_border_keeps_authored_alpha_mode() {
        let mut app = image_sdf_pipeline_app();
        let image = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::default());
        spawn_sdf_panel(
            &mut app,
            clipped_image_border_tree(image),
            StandardMaterial {
                alpha_mode: AlphaMode::Add,
                ..Default::default()
            },
        );
        settle_sdf_pipeline(&mut app);

        let records = sdf_records(&app);
        assert_eq!(records.len(), 1);
        let record = &records[0];

        assert_eq!(
            record.batch_key.pipeline_compatibility.alpha,
            BatchAlphaMode::Add
        );
        assert_ne!(
            record.batch_key.pipeline_compatibility.alpha,
            BatchAlphaMode::Blend
        );
        assert_eq!(
            record.clip_depth_nudge.to_bits(),
            record.draw_depth.clip_depth_nudge().get().to_bits()
        );
    }

    #[test]
    fn non_clipped_fill_border_stays_one_opaque_record() {
        let mut app = sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            bordered_surface_tree(Color::BLACK, IMAGE_BORDER_COLOR),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let records = sdf_records(&app);
        assert_eq!(records.len(), 1);
        let record = &records[0];

        assert!(matches!(
            record.fill_material,
            SdfPaintMaterial::Authored(_)
        ));
        assert!(matches!(
            record.border_material,
            SdfPaintMaterial::Authored(_)
        ));
        assert_eq!(record.draw_order.get(), 0);
        assert_eq!(
            record.batch_key.pipeline_compatibility.alpha,
            BatchAlphaMode::Opaque
        );
        assert_eq!(
            record.clip_depth_nudge.to_bits(),
            (record.draw_depth.clip_depth_nudge().get() - OPAQUE_FILL_DEPTH_PUSH_LAYERS).to_bits()
        );
    }

    #[test]
    fn clipping_border_routes_in_front_of_coplanar_image() {
        let mut app = image_sdf_pipeline_app();
        let image = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::default());
        spawn_sdf_panel(
            &mut app,
            clipped_image_border_tree(image),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let sdf_records = sdf_records(&app);
        let image_records = image_records(&app);
        assert_eq!(sdf_records.len(), 1);
        assert_eq!(image_records.len(), 1);
        let border_record = &sdf_records[0];
        let image_record = &image_records[0];

        assert!(matches!(
            border_record.fill_material,
            SdfPaintMaterial::NotAuthored
        ));
        assert!(matches!(
            border_record.border_material,
            SdfPaintMaterial::Authored(_)
        ));
        assert!(image_record.record_key.command_index < border_record.draw_order);
        assert_eq!(
            border_record.batch_key.pipeline_compatibility.alpha,
            BatchAlphaMode::Blend
        );
        assert_eq!(
            sdf_batch_alpha_mode(
                border_record.batch_key.pipeline_compatibility.alpha,
                border_record.batch_key.shadow,
            ),
            AlphaMode::Blend
        );
        assert_eq!(
            border_record.clip_depth_nudge.to_bits(),
            border_record.draw_depth.clip_depth_nudge().get().to_bits()
        );
        assert!(image_record.draw_depth.clip_depth_nudge().get() < border_record.clip_depth_nudge);
        assert!(image_record.draw_depth.oit_depth_offset().get() < border_record.oit_depth_offset);
        assert_eq!(
            image_record
                .draw_depth
                .z_index_rank()
                .screen_depth_bias()
                .get()
                .to_bits(),
            border_record
                .batch_key
                .z_index_rank
                .screen_depth_bias()
                .get()
                .to_bits()
        );
        assert_eq!(
            image_record
                .transform
                .transform_point3(Vec3::ZERO)
                .z
                .to_bits(),
            border_record
                .transform
                .transform_point3(Vec3::ZERO)
                .z
                .to_bits()
        );
    }

    #[test]
    fn clipped_filled_border_splits_fill_behind_and_border_in_front_of_image() {
        let mut app = image_sdf_pipeline_app();
        let image = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::default());
        spawn_sdf_panel(
            &mut app,
            filled_clipped_image_border_tree(image),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let sdf_records = sdf_records(&app);
        let image_records = image_records(&app);
        assert_eq!(sdf_records.len(), 2);
        assert_eq!(image_records.len(), 1);
        let image_record = &image_records[0];
        let Some(fill_record) = sdf_records.iter().find(|record| {
            matches!(record.fill_material, SdfPaintMaterial::Authored(_))
                && matches!(record.border_material, SdfPaintMaterial::NotAuthored)
        }) else {
            panic!("expected one fill-only SDF record");
        };
        let Some(border_record) = sdf_records.iter().find(|record| {
            matches!(record.fill_material, SdfPaintMaterial::NotAuthored)
                && matches!(record.border_material, SdfPaintMaterial::Authored(_))
        }) else {
            panic!("expected one border-only SDF record");
        };

        assert_eq!(fill_record.record_key.role, SdfRecordRole::Fill);
        assert_eq!(border_record.record_key.role, SdfRecordRole::Border);
        assert_ne!(fill_record.record_key, border_record.record_key);

        assert_eq!(
            fill_record.batch_key.pipeline_compatibility.alpha,
            BatchAlphaMode::Opaque
        );
        assert_eq!(
            fill_record.clip_depth_nudge.to_bits(),
            (fill_record.draw_depth.clip_depth_nudge().get() - OPAQUE_FILL_DEPTH_PUSH_LAYERS)
                .to_bits()
        );
        assert!(fill_record.oit_depth_offset <= image_record.draw_depth.oit_depth_offset().get());

        assert_eq!(
            border_record.batch_key.pipeline_compatibility.alpha,
            BatchAlphaMode::Blend
        );
        assert_eq!(
            border_record.clip_depth_nudge.to_bits(),
            border_record.draw_depth.clip_depth_nudge().get().to_bits()
        );
        assert!(image_record.draw_depth.oit_depth_offset().get() < border_record.oit_depth_offset);
        assert_eq!(
            image_record
                .draw_depth
                .z_index_rank()
                .screen_depth_bias()
                .get()
                .to_bits(),
            border_record
                .batch_key
                .z_index_rank
                .screen_depth_bias()
                .get()
                .to_bits()
        );
        assert_eq!(
            image_record
                .transform
                .transform_point3(Vec3::ZERO)
                .z
                .to_bits(),
            border_record
                .transform
                .transform_point3(Vec3::ZERO)
                .z
                .to_bits()
        );
    }

    #[test]
    fn clipped_material_filled_border_splits_fill_behind_and_border_in_front_of_image() {
        let mut app = image_sdf_pipeline_app();
        let image = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::default());
        let element_material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        spawn_sdf_panel(
            &mut app,
            material_filled_clipped_image_border_tree(image, element_material),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let sdf_records = sdf_records(&app);
        let image_records = image_records(&app);
        assert_eq!(sdf_records.len(), 2);
        assert_eq!(image_records.len(), 1);
        let image_record = &image_records[0];
        let Some(fill_record) = sdf_records.iter().find(|record| {
            matches!(record.fill_material, SdfPaintMaterial::Authored(_))
                && matches!(record.border_material, SdfPaintMaterial::NotAuthored)
        }) else {
            panic!("expected one fill-only SDF record");
        };
        let Some(border_record) = sdf_records.iter().find(|record| {
            matches!(record.fill_material, SdfPaintMaterial::NotAuthored)
                && matches!(record.border_material, SdfPaintMaterial::Authored(_))
        }) else {
            panic!("expected one border-only SDF record");
        };

        assert_eq!(fill_record.record_key.role, SdfRecordRole::Fill);
        assert_eq!(border_record.record_key.role, SdfRecordRole::Border);
        assert_ne!(fill_record.record_key, border_record.record_key);

        assert_eq!(
            fill_record.batch_key.pipeline_compatibility.alpha,
            BatchAlphaMode::Opaque
        );
        assert_eq!(
            fill_record.clip_depth_nudge.to_bits(),
            (fill_record.draw_depth.clip_depth_nudge().get() - OPAQUE_FILL_DEPTH_PUSH_LAYERS)
                .to_bits()
        );
        assert!(fill_record.oit_depth_offset <= image_record.draw_depth.oit_depth_offset().get());

        assert_eq!(
            border_record.batch_key.pipeline_compatibility.alpha,
            BatchAlphaMode::Blend
        );
        assert_eq!(
            border_record.clip_depth_nudge.to_bits(),
            border_record.draw_depth.clip_depth_nudge().get().to_bits()
        );
        assert!(image_record.draw_depth.oit_depth_offset().get() < border_record.oit_depth_offset);
        assert_eq!(
            image_record
                .draw_depth
                .z_index_rank()
                .screen_depth_bias()
                .get()
                .to_bits(),
            border_record
                .batch_key
                .z_index_rank
                .screen_depth_bias()
                .get()
                .to_bits()
        );
        assert_eq!(
            image_record
                .transform
                .transform_point3(Vec3::ZERO)
                .z
                .to_bits(),
            border_record
                .transform
                .transform_point3(Vec3::ZERO)
                .z
                .to_bits()
        );
    }

    #[test]
    fn normal_border_keeps_opaque_depth_push() {
        let mut app = sdf_pipeline_app();
        spawn_sdf_panel(&mut app, border_only_tree(), StandardMaterial::default());
        settle_sdf_pipeline(&mut app);

        let records = sdf_records(&app);
        assert_eq!(records.len(), 1);
        let record = &records[0];

        assert_eq!(
            record.batch_key.pipeline_compatibility.alpha,
            BatchAlphaMode::Opaque
        );
        assert_eq!(
            record.clip_depth_nudge.to_bits(),
            (record.draw_depth.clip_depth_nudge().get() - OPAQUE_FILL_DEPTH_PUSH_LAYERS).to_bits()
        );
    }

    #[test]
    fn color_only_sdf_updates_keep_the_batch_entity() {
        let mut app = sdf_pipeline_app();
        let panel = spawn_sdf_panel(
            &mut app,
            single_surface_tree(Color::WHITE),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let entity_before = live_sdf_batch_entities(&mut app);
        assert_eq!(entity_before.len(), 1);

        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, single_surface_tree(Color::srgb(0.7, 0.2, 0.1)))
                .is_ok()
        );
        settle_sdf_pipeline(&mut app);

        let entity_after = live_sdf_batch_entities(&mut app);
        let records_after = sdf_records(&app);
        assert_eq!(entity_after, entity_before);
        assert_eq!(records_after.len(), 1);
        assert_eq!(
            fill_row_color(&app, &records_after[0]),
            linear_color(Color::srgb(0.7, 0.2, 0.1))
        );
    }

    #[test]
    fn scalar_material_animation_keeps_sdf_batch_and_table_capacity_stable() {
        let mut app = sdf_pipeline_app();
        let first_material = animated_scalar_material(0, Color::srgb(0.2, 0.4, 0.8), 0.0);
        let second_material = animated_scalar_material(1, Color::srgb(0.8, 0.3, 0.2), 1.0);
        let initial_colors = [first_material.base_color, second_material.base_color];
        let first_handle = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(first_material);
        let second_handle = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(second_material);
        let panel = spawn_sdf_panel(
            &mut app,
            two_material_surface_tree(
                first_handle,
                initial_colors[0],
                second_handle,
                initial_colors[1],
            ),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let initial_records = sdf_records(&app);
        let initial_capacity = frame_material_capacity(&app);
        assert_eq!(live_sdf_batch_count(&mut app), 1);
        assert_eq!(initial_records.len(), 2);
        assert_eq!(authored_slot_count(&initial_records), 2);
        assert_authored_slots_are_distinct(&initial_records);
        assert_fill_row_colors(&app, &initial_records, &initial_colors);
        assert_eq!(
            frame_material_row_count(&app),
            authored_slot_count(&initial_records)
        );

        for frame in 2..6 {
            let first_material = animated_scalar_material(frame, Color::srgb(0.1, 0.5, 0.9), 0.2);
            let second_material =
                animated_scalar_material(frame + 1, Color::srgb(0.9, 0.4, 0.1), 0.8);
            let expected_colors = [first_material.base_color, second_material.base_color];
            let first_handle = app
                .world_mut()
                .resource_mut::<Assets<StandardMaterial>>()
                .add(first_material);
            let second_handle = app
                .world_mut()
                .resource_mut::<Assets<StandardMaterial>>()
                .add(second_material);
            assert!(
                app.world_mut()
                    .commands()
                    .set_tree(
                        panel,
                        two_material_surface_tree(
                            first_handle,
                            expected_colors[0],
                            second_handle,
                            expected_colors[1],
                        ),
                    )
                    .is_ok()
            );
            settle_sdf_pipeline(&mut app);

            let records = sdf_records(&app);
            assert_eq!(live_sdf_batch_count(&mut app), 1);
            assert_eq!(records.len(), 2);
            assert_eq!(authored_slot_count(&records), 2);
            assert_authored_slots_are_distinct(&records);
            assert_fill_row_colors(&app, &records, &expected_colors);
            assert_eq!(
                frame_material_row_count(&app),
                authored_slot_count(&records)
            );
            assert_eq!(frame_material_capacity(&app), initial_capacity);
        }
    }

    #[test]
    fn sdf_batch_key_splits_on_authored_policy_and_resource_compatibility() {
        let mut images = Assets::<Image>::default();
        let texture = images.add(Image::default());
        let baseline_material = StandardMaterial::default();
        let baseline_key = sdf_batch_key_for_material(&baseline_material, VisualShadow::None);
        let splitters = [
            (
                "alpha mode",
                sdf_batch_key_for_material(
                    &StandardMaterial {
                        alpha_mode: AlphaMode::Blend,
                        ..Default::default()
                    },
                    VisualShadow::None,
                ),
            ),
            (
                "double sided",
                sdf_batch_key_for_material(
                    &StandardMaterial {
                        double_sided: true,
                        ..Default::default()
                    },
                    VisualShadow::None,
                ),
            ),
            (
                "cull mode",
                sdf_batch_key_for_material(
                    &StandardMaterial {
                        cull_mode: Some(Face::Front),
                        ..Default::default()
                    },
                    VisualShadow::None,
                ),
            ),
            (
                "texture resource",
                sdf_batch_key_for_material(
                    &StandardMaterial {
                        base_color_texture: Some(texture),
                        ..Default::default()
                    },
                    VisualShadow::None,
                ),
            ),
            (
                "lighting mode",
                sdf_batch_key_for_material(
                    &StandardMaterial {
                        unlit: true,
                        ..Default::default()
                    },
                    VisualShadow::None,
                ),
            ),
            (
                "shadow policy",
                sdf_batch_key_for_material(&StandardMaterial::default(), VisualShadow::Cast),
            ),
        ];

        for (splitter, key) in splitters {
            assert_ne!(key, baseline_key, "{splitter} should split from baseline");
        }
    }

    fn sdf_batch_key_for_material(
        material: &StandardMaterial,
        shadow: VisualShadow,
    ) -> SdfBatchKey {
        SdfBatchKey {
            shadow,
            pipeline_compatibility: PipelineCompatibility::from(material),
            resource_compatibility: ResourceCompatibility::from(material),
            ..test_batch_key()
        }
    }

    #[test]
    fn cull_modes_are_distinct_sdf_compatibility_values() {
        let keys: HashSet<PipelineCompatibility> = [None, Some(Face::Back), Some(Face::Front)]
            .into_iter()
            .map(|cull_mode| {
                PipelineCompatibility::from(&StandardMaterial {
                    cull_mode,
                    ..Default::default()
                })
            })
            .collect();

        assert_eq!(keys.len(), 3);
    }

    #[test]
    fn record_and_material_rows_follow_live_sdf_surfaces() {
        let mut app = sdf_pipeline_app();
        let panel = spawn_sdf_panel(
            &mut app,
            stacked_surface_tree(Color::srgb(0.1, 0.1, 0.8), Color::srgb(0.2, 0.7, 0.3)),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let initial = sdf_records(&app);
        assert_eq!(initial.len(), 2);
        assert_authored_slots_are_distinct(&initial);
        assert_eq!(
            frame_material_live_row_count(&app),
            authored_slot_count(&initial)
        );

        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, single_surface_tree(Color::srgb(0.9, 0.2, 0.1)))
                .is_ok()
        );
        settle_sdf_pipeline(&mut app);

        let reduced = sdf_records(&app);
        assert_eq!(reduced.len(), 1);
        assert_authored_slots_are_distinct(&reduced);
        assert_eq!(
            frame_material_live_row_count(&app),
            authored_slot_count(&reduced)
        );
    }

    #[test]
    fn removed_and_respawned_panels_use_current_frame_material_rows() {
        let mut app = sdf_pipeline_app();
        let panel = spawn_sdf_panel(
            &mut app,
            bordered_surface_tree(Color::srgb(0.2, 0.4, 0.8), Color::srgb(0.9, 0.8, 0.2)),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let initial = sdf_records(&app);
        assert_eq!(initial.len(), 1);
        assert_eq!(authored_slot_count(&initial), 2);
        assert_eq!(frame_material_row_count(&app), 2);

        let _ = app.world_mut().despawn(panel);
        settle_sdf_pipeline(&mut app);

        assert!(sdf_records(&app).is_empty());
        assert_eq!(frame_material_row_count(&app), 0);

        spawn_sdf_panel(
            &mut app,
            bordered_surface_tree(Color::srgb(0.1, 0.8, 0.4), Color::srgb(0.8, 0.2, 0.9)),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let respawned = sdf_records(&app);
        assert_eq!(respawned.len(), 1);
        assert_eq!(authored_slot_count(&respawned), 2);
        assert_eq!(frame_material_row_count(&app), 2);
        assert_eq!(
            fill_row_color(&app, &respawned[0]),
            linear_color(Color::srgb(0.1, 0.8, 0.4))
        );
    }

    #[test]
    fn layer_changes_rekey_and_visibility_masks_sdf_records_same_frame() {
        let mut app = sdf_pipeline_app();
        let panel = spawn_sdf_panel(
            &mut app,
            single_surface_tree(Color::WHITE),
            StandardMaterial::default(),
        );
        app.world_mut()
            .entity_mut(panel)
            .insert(RenderLayers::layer(0));
        settle_sdf_pipeline(&mut app);

        let initial_records = sdf_records(&app);
        assert_eq!(initial_records.len(), 1);
        assert_eq!(
            initial_records[0].batch_key.layers,
            BatchRenderLayers(RenderLayers::layer(0))
        );
        assert_eq!(live_sdf_batch_count(&mut app), 1);

        app.world_mut()
            .entity_mut(panel)
            .insert(RenderLayers::layer(1));
        app.update();

        let relayered = sdf_records(&app);
        assert_eq!(relayered.len(), 1);
        assert_eq!(
            relayered[0].batch_key.layers,
            BatchRenderLayers(RenderLayers::layer(1))
        );
        assert_eq!(live_sdf_batch_count(&mut app), 1);

        app.world_mut().entity_mut(panel).insert(Visibility::Hidden);
        app.update();

        let hidden_records = sdf_records(&app);
        assert_eq!(hidden_records.len(), 1);
        assert_eq!(hidden_records[0].paint_mask.bits(), 0);
        assert_eq!(app.world().resource::<SdfBatchStore>().batches().count(), 1);
        assert_eq!(live_sdf_batch_count(&mut app), 1);
    }

    #[test]
    fn topology_churn_reuses_command_rows_without_stale_material_slots() {
        let survivor = Color::srgb(0.2, 0.7, 0.3);
        let added = Color::srgb(0.9, 0.2, 0.1);
        let mut app = sdf_pipeline_app();
        let panel = spawn_sdf_panel(
            &mut app,
            stacked_surface_tree(Color::srgb(0.1, 0.1, 0.8), survivor),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let initial = sdf_records(&app);
        assert_eq!(initial.len(), 2);
        assert_eq!(initial[0].draw_order, CommandIndex::from(0));
        assert_eq!(initial[1].draw_order, CommandIndex::from(1));

        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, topology_churn_tree(survivor, added))
                .is_ok()
        );
        app.update();

        let churned = sdf_records(&app);
        assert_eq!(churned.len(), 2);
        assert_eq!(churned[0].draw_order, CommandIndex::from(0));
        assert_eq!(churned[1].draw_order, CommandIndex::from(1));
        assert_eq!(fill_row_color(&app, &churned[0]), linear_color(survivor));
        assert_eq!(fill_row_color(&app, &churned[1]), linear_color(added));
        assert_eq!(app.world().resource::<SdfBatchStore>().batches().count(), 1);
    }

    #[test]
    fn in_batch_upload_order_follows_draw_depth_then_command_index() {
        let mut app = sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            stacked_surface_tree(
                Color::srgba(0.9, 0.1, 0.1, 0.5),
                Color::srgba(0.1, 0.1, 0.9, 0.5),
            ),
            StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                ..Default::default()
            },
        );
        settle_sdf_pipeline(&mut app);

        let records = single_sdf_batch_records(&app);

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].draw_order, CommandIndex::from(0));
        assert_eq!(records[1].draw_order, CommandIndex::from(1));
        assert!(
            records[0].draw_depth.draw_order_index() <= records[1].draw_depth.draw_order_index()
        );
        assert!(records[0].oit_depth_offset < records[1].oit_depth_offset);
    }

    #[test]
    fn equal_draw_depths_are_broken_by_draw_order() {
        let mut app = sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            stacked_surface_tree(
                Color::srgba(0.9, 0.1, 0.1, 0.5),
                Color::srgba(0.1, 0.1, 0.9, 0.5),
            ),
            StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                ..Default::default()
            },
        );
        settle_sdf_pipeline(&mut app);

        {
            let mut store = app.world_mut().resource_mut::<SdfBatchStore>();
            let (_, batch) = store
                .batches_mut()
                .next()
                .expect("the two surfaces should share one SDF batch");
            assert_eq!(batch.records.len(), 2);
            batch.records[1].draw_depth = batch.records[0].draw_depth;
            batch.records.swap(0, 1);
            batch.sort_records();
        }

        let records = single_sdf_batch_records(&app);

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].draw_order, CommandIndex::from(0));
        assert_eq!(records[1].draw_order, CommandIndex::from(1));
        assert_eq!(
            records[0].draw_depth.draw_order_index(),
            records[1].draw_depth.draw_order_index()
        );
    }

    #[test]
    fn deferred_sdf_record_sort_runs_once_per_dirty_batch() {
        let first = Color::srgb(0.9, 0.1, 0.1);
        let second = Color::srgb(0.1, 0.1, 0.9);
        let mut app = sdf_pipeline_app();
        let panel = spawn_sdf_panel(
            &mut app,
            stacked_surface_tree(first, second),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        reset_sdf_batch_sort_counts(&mut app);

        let updated = app
            .world_mut()
            .commands()
            .set_tree(panel, side_by_side_surface_tree(first, second));
        assert!(
            updated.is_ok(),
            "changing both SDF surface bounds should succeed"
        );
        app.update();

        assert_eq!(
            sdf_batch_sort_counts(&app),
            vec![1],
            "one dirty SDF batch sorts once after both changed records append",
        );

        reset_sdf_batch_sort_counts(&mut app);
        app.update();

        assert_eq!(
            sdf_batch_sort_counts(&app),
            vec![0],
            "a clean SDF batch performs no deferred sort",
        );
    }

    #[test]
    fn shader_sources_route_material_table_reads_through_helper() {
        let helper_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("shaders")
            .join("sdf_material_table.wgsl");
        let helper = fs::read_to_string(&helper_path).expect("SDF material helper should load");
        let body_range = function_body_range(&helper, "fn pbr_input_from_material_table");
        let body = &helper[body_range.clone()];
        let mut outside = String::new();
        outside.push_str(&helper[..body_range.start]);
        outside.push_str(&helper[body_range.end..]);

        assert!(helper.contains("fn pbr_input_from_material_table"));
        assert_eq!(outside.matches("material_table[").count(), 0);
        // Two mutually exclusive in-helper reads: the depth/shadow prepass reads
        // base_color only, the main pass reads the full row.
        assert_eq!(body.matches("material_table[").count(), 2);
        for path in crate_wgsl_files() {
            let source = fs::read_to_string(&path).expect("crate WGSL file should load");
            if path == helper_path {
                continue;
            }
            assert_eq!(
                source.matches("material_table[").count(),
                0,
                "{} should route material table reads through pbr_input_from_material_table",
                path.display()
            );
        }
        assert_ordered(
            body,
            &[
                "if !role_present",
                "if material_id == INVALID_GPU_MATERIAL_SLOT",
                "if material_id >= arrayLength(&material_table)",
                "material_table[material_index]",
            ],
        );
    }

    #[test]
    fn material_table_uv_transform_is_composed_before_texture_sampling() {
        let helper = include_str!("../shaders/sdf_material_table.wgsl");
        let body_range = function_body_range(helper, "fn pbr_input_from_material_table");
        let body = &helper[body_range];

        assert_ordered(
            body,
            &[
                "let values = material_table[material_index]",
                "sampled_input.uv = compute_material_sampled_uv(in.uv, values.uv_transform)",
                "pbr_input_from_standard_material(sampled_input, is_front)",
                "apply_material_slot_values(&pbr_input, values)",
            ],
        );
    }

    #[test]
    fn border_dominant_pixels_use_border_pbr_lighting_fields() {
        let shader = include_str!("../shaders/sdf_panel.wgsl");

        assert_ordered(
            shader,
            &[
                "let use_border_material",
                "pbr_input.material.emissive = border_pbr.material.emissive",
                "pbr_input.material.reflectance = border_pbr.material.reflectance",
                "pbr_input.material.perceptual_roughness = border_pbr.material.perceptual_roughness",
                "pbr_input.material.metallic = border_pbr.material.metallic",
                "pbr_input.material.ior = border_pbr.material.ior",
                "apply_pbr_lighting(pbr_input)",
            ],
        );
    }

    #[test]
    fn sdf_bindings_match_material_table_constants() {
        let rust_source = include_str!("fill_batch.rs");
        let helper = include_str!("../shaders/sdf_material_table.wgsl");
        let shader = include_str!("../shaders/sdf_panel.wgsl");
        let constants = include_str!("material_table.wgsl");

        assert_eq!(
            storage_binding_for_field(rust_source, "material_table"),
            MATERIAL_TABLE_BINDING
        );
        assert_eq!(
            storage_binding_for_field(rust_source, "records"),
            SDF_RENDER_RECORDS_BINDING
        );
        assert_eq!(
            storage_binding_for_field(rust_source, "mesh_records"),
            SDF_MESH_BINDING
        );
        assert_wgsl_storage_binding(helper, MATERIAL_TABLE_BINDING, "material_table");
        assert_wgsl_storage_binding(shader, SDF_RENDER_RECORDS_BINDING, "sdf_records");
        assert_wgsl_storage_binding(shader, SDF_MESH_BINDING, "sdf_mesh_records");
        assert_wgsl_binding_constant(constants, "MATERIAL_TABLE_BINDING", MATERIAL_TABLE_BINDING);
        assert_wgsl_binding_constant(
            constants,
            "SDF_RENDER_RECORDS_BINDING",
            SDF_RENDER_RECORDS_BINDING,
        );
        assert_wgsl_binding_constant(constants, "SDF_MESH_BINDING", SDF_MESH_BINDING);

        assert_sdf_shader_ref(SdfExtension::vertex_shader());
        assert_sdf_shader_ref(SdfExtension::fragment_shader());
        assert_sdf_shader_ref(SdfExtension::prepass_vertex_shader());
        assert_sdf_shader_ref(SdfExtension::prepass_fragment_shader());
        assert_sdf_shader_ref(SdfExtension::deferred_vertex_shader());
        assert_sdf_shader_ref(SdfExtension::deferred_fragment_shader());
        assert!(
            shader.contains(
                "#import hana_diegetic::sdf_material_table::pbr_input_from_material_table"
            )
        );
        assert_ordered(
            shader,
            &[
                "fn fill_alpha_for_prepass",
                "pbr_input_from_material_table(",
                "#ifdef PREPASS_PIPELINE",
                "@fragment",
            ],
        );
        assert_ordered(
            shader,
            &[
                "#else\n@fragment",
                "let fill_pbr = pbr_input_from_material_table(",
                "let border_pbr = pbr_input_from_material_table(",
            ],
        );
        // Specialized per-pipeline layout assertions stay in the GPU/visual pass because
        // `RenderPipelineDescriptor` layouts require a render-device-backed specialization path.
    }

    #[test]
    fn stripped_material_group_fallback_is_present_for_prepass_review() {
        let shader = include_str!("../shaders/sdf_panel.wgsl");
        let helper = include_str!("../shaders/sdf_material_table.wgsl");
        let mut descriptor = RenderPipelineDescriptor {
            fragment: Some(FragmentState::default()),
            ..Default::default()
        };

        add_sdf_stripped_material_group_def(&mut descriptor, SdfPipelineMode::MeshAttributes);
        assert!(descriptor.vertex.shader_defs.is_empty());
        assert!(
            descriptor
                .fragment
                .as_ref()
                .expect("fragment state should exist")
                .shader_defs
                .is_empty()
        );

        add_sdf_stripped_material_group_def(&mut descriptor, SdfPipelineMode::VertexPulled);

        let stripped = ShaderDefVal::from("SDF_STRIPPED_MATERIAL_GROUP");
        assert!(descriptor.vertex.shader_defs.contains(&stripped));
        assert!(
            descriptor
                .fragment
                .as_ref()
                .expect("fragment state should exist")
                .shader_defs
                .contains(&stripped)
        );
        assert!(shader.contains("#ifdef SDF_STRIPPED_MATERIAL_GROUP"));
        assert!(shader.contains("#ifdef PREPASS_PIPELINE"));
        assert!(helper.contains("#ifdef SDF_STRIPPED_MATERIAL_GROUP"));
        assert!(helper.contains("pbr_types::pbr_input_new()"));
        assert!(
            !helper[function_body_range(helper, "fn stripped_material_group_pbr_input")]
                .contains("pbr_input_from_standard_material")
        );
        assert_eq!(
            sdf_batch_alpha_mode(AlphaMode::Opaque.into(), VisualShadow::Cast),
            AlphaMode::Mask(0.0)
        );
    }

    fn test_batch_key() -> SdfBatchKey {
        SdfBatchKey {
            z_index:                0.into(),
            z_index_rank:           DrawZIndexRank::default(),
            batch_family:           DrawBatchFamily::SdfSurface,
            layers:                 BatchRenderLayers(RenderLayers::layer(0)),
            shadow:                 VisualShadow::Cast,
            contiguous_drawn_run:   ContiguousDrawnRun::default(),
            pipeline_compatibility: PipelineCompatibility::from(&StandardMaterial::default()),
            resource_compatibility: ResourceCompatibility::from(&StandardMaterial::default()),
        }
    }

    fn resolved_surface_for_test(
        command_index: usize,
        material: &Handle<StandardMaterial>,
        fill_authorship: SdfRoleAuthorship,
        border_authorship: SdfRoleAuthorship,
    ) -> ResolvedSdfSurface<'_> {
        ResolvedSdfSurface {
            panel_entity:    Entity::from_bits(1),
            command_index:   CommandIndex::from(command_index),
            element_index:   draw_order::ElementIndex::from(command_index),
            divider_ordinal: None,
            record_role:     SdfRecordRole::Whole,
            draw_depth:      draw_depth_for_test(command_index),
            fill_material:   ResolvedSdfMaterial {
                authorship:    fill_authorship,
                base_material: Some(material),
                color:         fill_authorship.is_authored().then_some(Color::WHITE),
            },
            border_material: ResolvedSdfMaterial {
                authorship:    border_authorship,
                base_material: Some(material),
                color:         border_authorship.is_authored().then_some(Color::BLACK),
            },
            local_center:    Vec2::ZERO,
            local_transform: Transform::IDENTITY,
            sdf_half_size:   Vec2::ONE,
            mesh_half_size:  Vec2::splat(1.01),
            corner_radii:    [0.0; 4],
            border_widths:   [0.0; 4],
            clip_rect:       Vec4::new(-1.0, -1.0, 1.0, 1.0),
            render_layers:   RenderLayers::layer(0),
            shadow_casting:  ShadowCasting::Off,
        }
    }

    fn slots_for_test(index: usize, material: &StandardMaterial) -> SdfRecordMaterialSlots {
        let slot_index = u32::try_from(index).expect("slot index fits in u32");
        SdfRecordMaterialSlots {
            fill:                   SdfPaintMaterial::Authored(
                MaterialSlotId::try_from(slot_index).expect("slot"),
            ),
            border:                 SdfPaintMaterial::NotAuthored,
            pipeline_compatibility: PipelineCompatibility::from(material),
            resource_compatibility: ResourceCompatibility::from(material),
        }
    }

    fn material_context_for_test(
        material: StandardMaterial,
    ) -> (
        Assets<StandardMaterial>,
        SdfMaterial,
        Handle<StandardMaterial>,
    ) {
        let mut materials = Assets::<StandardMaterial>::default();
        let default = materials.add(material::default_panel_material());
        let material = materials.add(material);
        (materials, SdfMaterial(default), material)
    }

    fn asset_server_for_test() -> AssetServer {
        let mut app = App::new();
        app.add_plugins(AssetPlugin::default());
        app.world().resource::<AssetServer>().clone()
    }

    fn draw_depth_for_test(index: usize) -> DrawCommandDepth {
        let commands: Vec<RenderCommand> = (0..=index)
            .map(|element_idx| RenderCommand {
                bounds: BoundingBox {
                    x:      0.0,
                    y:      0.0,
                    width:  1.0,
                    height: 1.0,
                },
                kind: RenderCommandKind::Rectangle {
                    color:  Color::WHITE,
                    source: RectangleSource::Background,
                },
                element_idx,
                z_index: DrawZIndex(0),
            })
            .collect();
        draw_order::DrawOrder::from_commands(&commands)
            .depth_for(index)
            .expect("draw command should have depth")
    }

    // ── Widget visual-slot overrides ────────────────────────────────────────

    fn widget_sdf_pipeline_app() -> App {
        let mut app = sdf_pipeline_app();
        app.add_plugins(WidgetsPlugin);
        app
    }

    fn slotted_widget_fill_tree(peer_z_index: i8) -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::column().width(Sizing::GROW).height(Sizing::GROW),
            |builder| {
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(WIDGET_FILL_COLOR)
                        .button("styled")
                        .visual_slot(TEST_SLOT),
                    |_| {},
                );
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(PEER_FILL_COLOR)
                        .z_index(peer_z_index),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    enum FocusedBorderState {
        Absent,
        Authored,
    }

    enum BorderCommandState {
        Absent,
        Authored,
    }

    enum SiblingBorderState {
        Absent,
        Authored,
    }

    fn focused_border_insertion_tree(focused_border_state: FocusedBorderState) -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::column().width(Sizing::GROW).height(Sizing::GROW),
            |builder| {
                let first = El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .background(WIDGET_FILL_COLOR)
                    .button("stateful");
                let first = match focused_border_state {
                    FocusedBorderState::Absent => first,
                    FocusedBorderState::Authored => first.focused(BorderColor(STATE_BORDER_COLOR)),
                };
                builder.with(first, |_| {});
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(PEER_FILL_COLOR),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    fn border_command_insertion_tree(border_command_state: BorderCommandState) -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::column().width(Sizing::GROW).height(Sizing::GROW),
            |builder| {
                let first = El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .background(WIDGET_FILL_COLOR);
                let first = match border_command_state {
                    BorderCommandState::Absent => first,
                    BorderCommandState::Authored => {
                        first.border(Border::all(Mm(1.0), STATE_BORDER_COLOR))
                    },
                };
                builder.with(first, |_| {});
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(PEER_FILL_COLOR),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    fn divider_key_tree(sibling_border_state: SiblingBorderState) -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::column()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(WIDGET_FILL_COLOR)
                .child_divider(ChildDivider::new(Mm(1.0), CHILD_DIVIDER_COLOR)),
            |builder| {
                let first = El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .background(PEER_FILL_COLOR);
                let first = match sibling_border_state {
                    SiblingBorderState::Absent => first,
                    SiblingBorderState::Authored => {
                        first.border(Border::all(Mm(1.0), STATE_BORDER_COLOR))
                    },
                };
                builder.with(first, |_| {});
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(WIDGET_LABEL_FILL_COLOR),
                    |_| {},
                );
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(WIDGET_TRACK_FILL_COLOR),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    fn styled_divider_tree(parent_material: Handle<StandardMaterial>) -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::column()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(WIDGET_FILL_COLOR)
                .material(parent_material)
                .corner_radius(PARENT_ELEMENT_CORNER_RADIUS)
                .child_divider(ChildDivider::new(Mm(1.0), CHILD_DIVIDER_COLOR))
                .button("styled")
                .visual_slot(TEST_SLOT),
            |builder| {
                for color in [
                    PEER_FILL_COLOR,
                    WIDGET_LABEL_FILL_COLOR,
                    WIDGET_TRACK_FILL_COLOR,
                ] {
                    builder.with(
                        El::new()
                            .width(Sizing::GROW)
                            .height(Sizing::GROW)
                            .background(color),
                        |_| {},
                    );
                }
            },
        );
        builder.build()
    }

    fn slotted_widget_label_track_thumb_fill_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::column()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .button("styled"),
            |builder| {
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(WIDGET_LABEL_FILL_COLOR)
                        .visual_slot(WIDGET_LABEL_SLOT),
                    |_| {},
                );
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(WIDGET_TRACK_FILL_COLOR)
                        .visual_slot(WIDGET_TRACK_SLOT),
                    |_| {},
                );
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(WIDGET_THUMB_FILL_COLOR)
                        .visual_slot(WIDGET_THUMB_SLOT),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    fn slotted_widget_bordered_fill_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(WIDGET_FILL_COLOR)
                .border(Border::all(Mm(IMAGE_BORDER_WIDTH_MM), IMAGE_BORDER_COLOR))
                .button("styled")
                .visual_slot(TEST_SLOT),
            |_| {},
        );
        builder.build()
    }

    fn slotted_widget_border_only_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .border(Border::all(Mm(IMAGE_BORDER_WIDTH_MM), IMAGE_BORDER_COLOR))
                .button("styled")
                .visual_slot(TEST_SLOT),
            |_| {},
        );
        builder.build()
    }

    fn styled_widget(app: &mut App) -> Entity {
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<PanelWidget>>();
        let widgets: Vec<Entity> = query.iter(app.world()).collect();
        assert_eq!(widgets.len(), 1, "expected exactly one reified widget");
        widgets[0]
    }

    fn set_slot_override(app: &mut App, widget: Entity, value: VisualSlotOverride) {
        set_slot_overrides(app, widget, [(TEST_SLOT, value)]);
    }

    fn set_slot_overrides(
        app: &mut App,
        widget: Entity,
        values: impl IntoIterator<Item = (VisualSlotId, VisualSlotOverride)>,
    ) {
        let mut entity = app.world_mut().entity_mut(widget);
        if let Some(mut overrides) = entity.get_mut::<WidgetVisualOverrides>() {
            for (slot, value) in values {
                overrides.set(slot, value);
            }
        } else {
            let mut overrides = WidgetVisualOverrides::default();
            for (slot, value) in values {
                overrides.set(slot, value);
            }
            entity.insert(overrides);
        }
    }

    fn sdf_record_key_for_fill_color(app: &App, color: Color) -> SdfRecordKey {
        sdf_records(app)
            .into_iter()
            .find(|record| fill_row_color(app, record) == linear_color(color))
            .map(|record| record.record_key)
            .expect("the widget part fill record should exist")
    }

    fn sdf_record_batch_snapshot(
        app: &App,
        record_key: SdfRecordKey,
    ) -> (SdfBatchKey, Entity, ResolvedSdfBatchRecord) {
        app.world()
            .resource::<SdfBatchStore>()
            .batches()
            .find_map(|(batch_key, batch)| {
                batch
                    .records()
                    .iter()
                    .find(|record| record.record_key == record_key)
                    .map(|record| {
                        (
                            batch_key.clone(),
                            batch.entity.expect("widget part batch should be spawned"),
                            record.clone(),
                        )
                    })
            })
            .expect("the widget part record should remain retained")
    }

    fn clear_slot_override(app: &mut App, widget: Entity) {
        let mut entity = app.world_mut().entity_mut(widget);
        if let Some(mut overrides) = entity.get_mut::<WidgetVisualOverrides>() {
            overrides.clear(TEST_SLOT);
        }
    }

    fn sdf_upload_count(app: &App) -> usize {
        app.world()
            .resource::<DiegeticPerfStats>()
            .panel_geometry
            .sdf_uploads
            .get()
    }

    fn assert_ignores_picking(app: &App, entity: Entity) {
        let pickable = app
            .world()
            .get::<Pickable>(entity)
            .expect("batch entity should carry Pickable::IGNORE");
        assert!(!pickable.is_hoverable);
        assert!(!pickable.should_block_lower);
    }

    /// Widget/panel state that a state-only visual override must not touch.
    struct WidgetGeometrySnapshot {
        transform:      Transform,
        anchor_rect:    WidgetAnchorRect,
        widget_records: Vec<ComputedWidgetRecord>,
        panel_tick:     Tick,
        computed_tick:  Tick,
    }

    fn widget_geometry_snapshot(
        app: &App,
        panel: Entity,
        widget: Entity,
    ) -> WidgetGeometrySnapshot {
        WidgetGeometrySnapshot {
            transform:      *app
                .world()
                .get::<Transform>(widget)
                .expect("widget should carry a transform"),
            anchor_rect:    *app
                .world()
                .get::<WidgetAnchorRect>(widget)
                .expect("widget should carry an anchor rect"),
            widget_records: app
                .world()
                .get::<ComputedDiegeticPanel>(panel)
                .expect("panel should have computed output")
                .widget_records()
                .to_vec(),
            panel_tick:     app
                .world()
                .entity(panel)
                .get_ref::<DiegeticPanel>()
                .expect("panel role should be live")
                .last_changed(),
            computed_tick:  app
                .world()
                .entity(panel)
                .get_ref::<ComputedDiegeticPanel>()
                .expect("panel should have computed output")
                .last_changed(),
        }
    }

    fn assert_widget_geometry_unchanged(
        app: &App,
        panel: Entity,
        widget: Entity,
        snapshot: &WidgetGeometrySnapshot,
    ) {
        let current = widget_geometry_snapshot(app, panel, widget);
        assert_eq!(
            current.panel_tick, snapshot.panel_tick,
            "an override must not fire Changed<DiegeticPanel>",
        );
        assert_eq!(
            current.computed_tick, snapshot.computed_tick,
            "an override must not fire Changed<ComputedDiegeticPanel> or solve geometry",
        );
        assert_eq!(
            current.transform, snapshot.transform,
            "an override must not move the widget transform",
        );
        assert_eq!(
            current.anchor_rect, snapshot.anchor_rect,
            "an override must not change the widget hit rectangle",
        );
        assert_eq!(
            current.widget_records, snapshot.widget_records,
            "an override must not change widget rects or interaction rank",
        );
    }

    #[test]
    fn same_key_offset_override_dirties_only_the_referenced_sdf_batch() {
        let mut app = widget_sdf_pipeline_app();
        let panel = spawn_sdf_panel(
            &mut app,
            slotted_widget_fill_tree(1),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        app.update();
        assert_eq!(
            sdf_upload_count(&app),
            0,
            "settled pipeline should be quiet"
        );
        let records_before = sdf_records(&app);
        assert_eq!(records_before.len(), 2);
        assert_eq!(
            app.world().resource::<SdfBatchStore>().batches().count(),
            2,
            "distinct z-indexes should split the two fills into two batches",
        );

        let widget = styled_widget(&mut app);
        let snapshot = widget_geometry_snapshot(&app, panel, widget);
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_offset(OVERRIDE_OFFSET),
        );
        app.update();

        assert_eq!(
            sdf_upload_count(&app),
            1,
            "only the referenced batch should re-upload",
        );
        let records_after = sdf_records(&app);
        assert_eq!(
            records_after[0].local_transform.translation,
            records_before[0].local_transform.translation + OVERRIDE_OFFSET.extend(0.0),
        );
        assert_eq!(records_after[1], records_before[1], "peer record untouched");
        assert_widget_geometry_unchanged(&app, panel, widget, &snapshot);

        // Author the identical override a second time: the changed component
        // re-dispatches, but the routes rebuild equal records, so no retained
        // record, batch entity, or GPU row changes.
        let entities_before = live_sdf_batch_entities(&mut app);
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_offset(OVERRIDE_OFFSET),
        );
        app.update();
        assert_eq!(
            sdf_upload_count(&app),
            0,
            "a repeated identical override is a no-op",
        );
        assert_eq!(sdf_records(&app), records_after);
        assert_eq!(live_sdf_batch_entities(&mut app), entities_before);
        assert_widget_geometry_unchanged(&app, panel, widget, &snapshot);
    }

    #[test]
    fn fill_color_override_patches_material_row_without_rekeying() {
        let mut app = widget_sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            slotted_widget_fill_tree(0),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        assert_eq!(live_sdf_batch_count(&mut app), 1);
        let records = sdf_records(&app);
        assert_eq!(records.len(), 2);
        assert_fill_row_colors(&app, &records, &[WIDGET_FILL_COLOR, PEER_FILL_COLOR]);
        let entities_before = live_sdf_batch_entities(&mut app);

        let widget = styled_widget(&mut app);
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_fill_color(SLOT_OVERRIDE_COLOR),
        );
        app.update();

        let records = sdf_records(&app);
        assert_fill_row_colors(&app, &records, &[SLOT_OVERRIDE_COLOR, PEER_FILL_COLOR]);
        assert_eq!(live_sdf_batch_entities(&mut app), entities_before);
        assert_eq!(
            sdf_upload_count(&app),
            0,
            "a fill color override flows through the frame material table, not the record buffer",
        );

        clear_slot_override(&mut app, widget);
        app.update();
        let records = sdf_records(&app);
        assert_fill_row_colors(&app, &records, &[WIDGET_FILL_COLOR, PEER_FILL_COLOR]);
    }

    #[test]
    fn focused_border_authoring_preserves_sibling_material_source_key() {
        let mut app = widget_sdf_pipeline_app();
        let panel = spawn_sdf_panel(
            &mut app,
            focused_border_insertion_tree(FocusedBorderState::Absent),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let expected = SdfMaterialSourceKey {
            panel,
            element: ElementIndex::from(3),
            divider_ordinal: None,
            role: SdfMaterialRole::Fill,
        };
        let expected_record_key = SdfRecordKey {
            panel,
            element: ElementIndex::from(3),
            divider_ordinal: None,
            role: SdfRecordRole::Whole,
        };
        let records = sdf_records(&app);
        let initial_peer_slot = records
            .iter()
            .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
            .and_then(|record| match record.fill_material {
                SdfPaintMaterial::Authored(slot) => Some(slot),
                SdfPaintMaterial::NotAuthored => None,
            });
        assert_eq!(
            records
                .iter()
                .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
                .map(|record| record.fill_source),
            Some(expected)
        );
        assert_eq!(
            records
                .iter()
                .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
                .map(|record| record.record_key),
            Some(expected_record_key)
        );
        assert_eq!(
            records
                .iter()
                .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
                .map(|record| record.draw_order),
            Some(CommandIndex::from(1))
        );
        assert_eq!(
            authored_fill_draw_orders(&app),
            vec![CommandIndex::from(0), CommandIndex::from(1)]
        );
        assert!(initial_peer_slot.is_some());

        let updated = app.world_mut().commands().set_tree(
            panel,
            focused_border_insertion_tree(FocusedBorderState::Authored),
        );
        assert!(
            updated.is_ok(),
            "focused-border replacement should be accepted"
        );
        settle_sdf_pipeline(&mut app);

        let records = sdf_records(&app);
        let updated_peer_slot = records
            .iter()
            .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
            .and_then(|record| match record.fill_material {
                SdfPaintMaterial::Authored(slot) => Some(slot),
                SdfPaintMaterial::NotAuthored => None,
            });
        assert_eq!(
            records
                .iter()
                .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
                .map(|record| record.fill_source),
            Some(expected)
        );
        assert_eq!(
            records
                .iter()
                .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
                .map(|record| record.record_key),
            Some(expected_record_key)
        );
        assert_eq!(
            records
                .iter()
                .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
                .map(|record| record.draw_order),
            Some(CommandIndex::from(2))
        );
        assert_eq!(
            authored_fill_draw_orders(&app),
            vec![CommandIndex::from(0), CommandIndex::from(2)]
        );
        assert_eq!(updated_peer_slot, initial_peer_slot);
    }

    #[test]
    fn inserted_border_command_preserves_existing_material_source_key() {
        let mut app = sdf_pipeline_app();
        let panel = spawn_sdf_panel(
            &mut app,
            border_command_insertion_tree(BorderCommandState::Absent),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let expected = SdfMaterialSourceKey {
            panel,
            element: ElementIndex::from(3),
            divider_ordinal: None,
            role: SdfMaterialRole::Fill,
        };
        let expected_record_key = SdfRecordKey {
            panel,
            element: ElementIndex::from(3),
            divider_ordinal: None,
            role: SdfRecordRole::Whole,
        };
        let records = sdf_records(&app);
        let initial_peer_slot = records
            .iter()
            .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
            .and_then(|record| match record.fill_material {
                SdfPaintMaterial::Authored(slot) => Some(slot),
                SdfPaintMaterial::NotAuthored => None,
            });
        assert_eq!(
            records
                .iter()
                .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
                .map(|record| record.fill_source),
            Some(expected)
        );
        assert_eq!(
            records
                .iter()
                .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
                .map(|record| record.record_key),
            Some(expected_record_key)
        );
        assert_eq!(
            records
                .iter()
                .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
                .map(|record| record.draw_order),
            Some(CommandIndex::from(1))
        );
        assert_eq!(
            authored_fill_draw_orders(&app),
            vec![CommandIndex::from(0), CommandIndex::from(1)]
        );
        assert!(initial_peer_slot.is_some());

        let updated = app.world_mut().commands().set_tree(
            panel,
            border_command_insertion_tree(BorderCommandState::Authored),
        );
        assert!(updated.is_ok(), "border replacement should be accepted");
        settle_sdf_pipeline(&mut app);

        let records = sdf_records(&app);
        let updated_peer_slot = records
            .iter()
            .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
            .and_then(|record| match record.fill_material {
                SdfPaintMaterial::Authored(slot) => Some(slot),
                SdfPaintMaterial::NotAuthored => None,
            });
        assert_eq!(
            records
                .iter()
                .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
                .map(|record| record.fill_source),
            Some(expected)
        );
        assert_eq!(
            records
                .iter()
                .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
                .map(|record| record.record_key),
            Some(expected_record_key)
        );
        assert_eq!(
            records
                .iter()
                .find(|record| fill_row_color(&app, record) == linear_color(PEER_FILL_COLOR))
                .map(|record| record.draw_order),
            Some(CommandIndex::from(2))
        );
        assert_eq!(
            authored_fill_draw_orders(&app),
            vec![CommandIndex::from(0), CommandIndex::from(2)]
        );
        assert_eq!(updated_peer_slot, initial_peer_slot);
    }

    #[test]
    fn divider_material_source_keys_are_distinct_and_stable() {
        let mut app = sdf_pipeline_app();
        let panel = spawn_sdf_panel(
            &mut app,
            divider_key_tree(SiblingBorderState::Absent),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let parent_fill = SdfMaterialSourceKey {
            panel,
            element: ElementIndex::from(1),
            divider_ordinal: None,
            role: SdfMaterialRole::Fill,
        };
        let expected = HashSet::from([
            SdfMaterialSourceKey {
                panel,
                element: ElementIndex::from(1),
                divider_ordinal: Some(0),
                role: SdfMaterialRole::Fill,
            },
            SdfMaterialSourceKey {
                panel,
                element: ElementIndex::from(1),
                divider_ordinal: Some(1),
                role: SdfMaterialRole::Fill,
            },
        ]);
        let records = sdf_records(&app);
        let divider_keys: HashSet<_> = records
            .iter()
            .map(|record| record.fill_source)
            .filter(|source| source.divider_ordinal.is_some())
            .collect();
        let divider_slots: HashSet<_> = records
            .iter()
            .filter(|record| record.fill_source.divider_ordinal.is_some())
            .filter_map(|record| match record.fill_material {
                SdfPaintMaterial::Authored(slot) => Some(slot),
                SdfPaintMaterial::NotAuthored => None,
            })
            .collect();
        let parent_fill_slot = records.iter().find_map(|record| {
            match (record.fill_source == parent_fill, record.fill_material) {
                (true, SdfPaintMaterial::Authored(slot)) => Some(slot),
                _ => None,
            }
        });
        let initial_divider_command_indices: HashMap<_, _> = records
            .iter()
            .map(|record| (record.fill_source, record.draw_order))
            .filter(|(source, _)| source.divider_ordinal.is_some())
            .collect();
        assert_eq!(divider_keys, expected);
        assert!(parent_fill_slot.is_some());
        assert!(
            divider_slots
                .iter()
                .all(|divider_slot| Some(*divider_slot) != parent_fill_slot)
        );
        assert_eq!(divider_slots.len(), 2);

        let updated = app
            .world_mut()
            .commands()
            .set_tree(panel, divider_key_tree(SiblingBorderState::Authored));
        assert!(updated.is_ok(), "divider replacement should be accepted");
        settle_sdf_pipeline(&mut app);

        let divider_keys: HashSet<_> = sdf_records(&app)
            .iter()
            .map(|record| record.fill_source)
            .filter(|source| source.divider_ordinal.is_some())
            .collect();
        assert_eq!(divider_keys, expected);
        let updated_divider_command_indices: HashMap<_, _> = sdf_records(&app)
            .iter()
            .map(|record| (record.fill_source, record.draw_order))
            .filter(|(source, _)| source.divider_ordinal.is_some())
            .collect();
        assert!(
            expected.iter().all(|key| {
                matches!(
                    (
                        initial_divider_command_indices.get(key),
                        updated_divider_command_indices.get(key),
                    ),
                    (Some(before), Some(after)) if after > before
                )
            }),
            "the inserted sibling border must shift both divider commands",
        );
    }

    #[test]
    fn dividers_ignore_parent_material_and_visual_slot_overrides() {
        let mut app = widget_sdf_pipeline_app();
        let parent_material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                metallic: PARENT_ELEMENT_METALLIC,
                ..Default::default()
            });
        spawn_sdf_panel(
            &mut app,
            styled_divider_tree(parent_material),
            StandardMaterial {
                metallic: DIVIDER_PANEL_METALLIC,
                ..Default::default()
            },
        );
        settle_sdf_pipeline(&mut app);

        let widget = styled_widget(&mut app);
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_fill_color(SLOT_OVERRIDE_COLOR),
        );
        app.update();

        let records = sdf_records(&app);
        let Some(parent_fill) = records.iter().find(|record| {
            record.fill_source.divider_ordinal.is_none()
                && fill_row_color(&app, record) == linear_color(SLOT_OVERRIDE_COLOR)
        }) else {
            panic!();
        };
        let dividers: Vec<_> = records
            .iter()
            .filter(|record| record.fill_source.divider_ordinal.is_some())
            .collect();

        assert_eq!(dividers.len(), 2);
        assert_eq!(
            fill_row_color(&app, parent_fill),
            linear_color(SLOT_OVERRIDE_COLOR)
        );
        assert_same_f32(
            fill_row_metallic(&app, parent_fill),
            PARENT_ELEMENT_METALLIC,
        );
        assert_ne!(parent_fill.corner_radii.value, Vec4::ZERO);
        for divider in dividers {
            assert_eq!(
                fill_row_color(&app, divider),
                linear_color(CHILD_DIVIDER_COLOR)
            );
            assert_same_f32(fill_row_metallic(&app, divider), DIVIDER_PANEL_METALLIC);
            assert_eq!(divider.corner_radii.value, Vec4::ZERO);
        }
    }

    #[test]
    fn compatible_material_override_keeps_the_widget_batch_and_entities() {
        let mut app = widget_sdf_pipeline_app();
        let compatible_material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                metallic: 0.6,
                ..Default::default()
            });
        spawn_sdf_panel(
            &mut app,
            slotted_widget_fill_tree(1),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let entities_before = live_sdf_batch_entities(&mut app);
        let widget_key_before = app
            .world()
            .resource::<SdfBatchStore>()
            .batches()
            .find(|(key, _)| key.z_index == 0.into())
            .map(|(key, _)| key.clone())
            .expect("the widget fill batch should exist");
        let widget = styled_widget(&mut app);

        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_material(compatible_material),
        );
        app.update();

        let widget_key_after = app
            .world()
            .resource::<SdfBatchStore>()
            .batches()
            .find(|(key, _)| key.z_index == 0.into())
            .map(|(key, _)| key.clone())
            .expect("the widget fill batch should remain");
        assert_eq!(widget_key_after, widget_key_before);
        assert_eq!(live_sdf_batch_entities(&mut app), entities_before);
    }

    #[test]
    fn compatible_material_overrides_keep_all_widget_part_batches_and_entities() {
        let mut app = widget_sdf_pipeline_app();
        let compatible_material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                metallic: 0.6,
                ..Default::default()
            });
        spawn_sdf_panel(
            &mut app,
            slotted_widget_label_track_thumb_fill_tree(),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        let widget = styled_widget(&mut app);
        let part_keys = [
            sdf_record_key_for_fill_color(&app, WIDGET_LABEL_FILL_COLOR),
            sdf_record_key_for_fill_color(&app, WIDGET_TRACK_FILL_COLOR),
            sdf_record_key_for_fill_color(&app, WIDGET_THUMB_FILL_COLOR),
        ];
        let batches_before =
            part_keys.map(|record_key| sdf_record_batch_snapshot(&app, record_key));
        let entities_before = live_sdf_batch_entities(&mut app);

        set_slot_overrides(
            &mut app,
            widget,
            [
                (
                    WIDGET_LABEL_SLOT,
                    VisualSlotOverride::default().with_material(compatible_material.clone()),
                ),
                (
                    WIDGET_TRACK_SLOT,
                    VisualSlotOverride::default().with_material(compatible_material.clone()),
                ),
                (
                    WIDGET_THUMB_SLOT,
                    VisualSlotOverride::default().with_material(compatible_material),
                ),
            ],
        );
        app.update();

        let batches_after = part_keys.map(|record_key| sdf_record_batch_snapshot(&app, record_key));
        for ((key_before, entity_before, _), (key_after, entity_after, _)) in
            batches_before.iter().zip(&batches_after)
        {
            assert_eq!(key_after, key_before);
            assert_eq!(entity_after, entity_before);
        }
        assert_eq!(live_sdf_batch_entities(&mut app), entities_before);
    }

    #[test]
    fn incompatible_material_overrides_migrate_only_changed_widget_parts() {
        let mut app = widget_sdf_pipeline_app();
        let incompatible_material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                ..Default::default()
            });
        spawn_sdf_panel(
            &mut app,
            slotted_widget_label_track_thumb_fill_tree(),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        let widget = styled_widget(&mut app);
        let label_key = sdf_record_key_for_fill_color(&app, WIDGET_LABEL_FILL_COLOR);
        let track_key = sdf_record_key_for_fill_color(&app, WIDGET_TRACK_FILL_COLOR);
        let thumb_key = sdf_record_key_for_fill_color(&app, WIDGET_THUMB_FILL_COLOR);
        let label_before = sdf_record_batch_snapshot(&app, label_key);
        let track_before = sdf_record_batch_snapshot(&app, track_key);
        let thumb_before = sdf_record_batch_snapshot(&app, thumb_key);

        set_slot_overrides(
            &mut app,
            widget,
            [(
                WIDGET_THUMB_SLOT,
                VisualSlotOverride::default().with_material(incompatible_material),
            )],
        );
        app.update();

        let label_after = sdf_record_batch_snapshot(&app, label_key);
        let track_after = sdf_record_batch_snapshot(&app, track_key);
        let thumb_after = sdf_record_batch_snapshot(&app, thumb_key);
        assert_eq!(label_after, label_before);
        assert_eq!(track_after, track_before);
        assert_ne!(thumb_after.0, thumb_before.0);
        assert_ne!(thumb_after.1, thumb_before.1);
        assert_eq!(sdf_records(&app).len(), 3);
    }

    #[test]
    fn fill_and_border_color_overrides_recolor_their_authored_roles() {
        let mut app = widget_sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            slotted_widget_bordered_fill_tree(),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        let records = sdf_records(&app);
        assert_eq!(records.len(), 1);
        assert_eq!(
            fill_row_color(&app, &records[0]),
            linear_color(WIDGET_FILL_COLOR),
        );
        assert_eq!(
            border_row_color(&app, &records[0]),
            linear_color(IMAGE_BORDER_COLOR),
        );

        let widget = styled_widget(&mut app);
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default()
                .with_fill_color(SLOT_OVERRIDE_COLOR)
                .with_border_color(SLOT_OVERRIDE_COLOR),
        );
        app.update();

        let records = sdf_records(&app);
        assert_eq!(
            fill_row_color(&app, &records[0]),
            linear_color(SLOT_OVERRIDE_COLOR),
        );
        assert_eq!(
            border_row_color(&app, &records[0]),
            linear_color(SLOT_OVERRIDE_COLOR),
            "each SDF role receives its own color override",
        );

        clear_slot_override(&mut app, widget);
        app.update();
        let records = sdf_records(&app);
        assert_eq!(
            fill_row_color(&app, &records[0]),
            linear_color(WIDGET_FILL_COLOR),
        );
        assert_eq!(
            border_row_color(&app, &records[0]),
            linear_color(IMAGE_BORDER_COLOR),
        );
    }

    #[test]
    fn role_specific_colors_recolor_fill_and_border_independently() {
        let mut app = widget_sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            slotted_widget_bordered_fill_tree(),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        let records = sdf_records(&app);
        assert_eq!(records.len(), 1);

        let widget = styled_widget(&mut app);
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default()
                .with_fill_color(SLOT_OVERRIDE_COLOR)
                .with_border_color(PEER_FILL_COLOR),
        );
        app.update();

        let records = sdf_records(&app);
        assert_eq!(
            fill_row_color(&app, &records[0]),
            linear_color(SLOT_OVERRIDE_COLOR),
        );
        assert_eq!(
            border_row_color(&app, &records[0]),
            linear_color(PEER_FILL_COLOR),
            "the border role takes its own override color",
        );

        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_fill_color(SLOT_OVERRIDE_COLOR),
        );
        app.update();
        let records = sdf_records(&app);
        assert_eq!(
            fill_row_color(&app, &records[0]),
            linear_color(SLOT_OVERRIDE_COLOR),
        );
        assert_eq!(
            border_row_color(&app, &records[0]),
            linear_color(IMAGE_BORDER_COLOR),
            "a fill-only override leaves the authored border color",
        );

        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default()
                .with_fill_color(SLOT_OVERRIDE_COLOR)
                .with_border_color(PEER_FILL_COLOR),
        );
        app.update();
        let records = sdf_records(&app);
        assert_eq!(
            fill_row_color(&app, &records[0]),
            linear_color(SLOT_OVERRIDE_COLOR),
            "the fill color remains independent from the border color",
        );
        assert_eq!(
            border_row_color(&app, &records[0]),
            linear_color(PEER_FILL_COLOR),
            "the border color updates independently from the fill color",
        );
    }

    #[test]
    fn role_specific_colors_never_author_a_missing_fill_role() {
        let mut app = widget_sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            slotted_widget_border_only_tree(),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let widget = styled_widget(&mut app);
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default()
                .with_fill_color(SLOT_OVERRIDE_COLOR)
                .with_border_color(PEER_FILL_COLOR),
        );
        app.update();

        let records = sdf_records(&app);
        assert_eq!(
            records[0].fill_material,
            SdfPaintMaterial::NotAuthored,
            "a fill-role color must not author a missing fill role",
        );
        assert_eq!(
            border_row_color(&app, &records[0]),
            linear_color(PEER_FILL_COLOR),
        );
    }

    #[test]
    fn border_width_override_rewrites_widths_without_moving_outer_bounds() {
        let mut app = widget_sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            slotted_widget_bordered_fill_tree(),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        let records = sdf_records(&app);
        assert_eq!(records.len(), 1);
        let authored_widths = records[0].border_widths.value;
        let authored_half_size = records[0].half_size.value;
        let widened = authored_widths * 3.0;

        let widget = styled_widget(&mut app);
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_border_widths(widened.to_array()),
        );
        app.update();

        let records = sdf_records(&app);
        assert_eq!(records[0].border_widths.value, widened);
        assert_eq!(
            records[0].half_size.value, authored_half_size,
            "the border grows inward, so the record's outer bounds stay solved",
        );

        clear_slot_override(&mut app, widget);
        app.update();
        let records = sdf_records(&app);
        assert_eq!(
            records[0].border_widths.value, authored_widths,
            "clearing the override restores the authored widths",
        );
    }

    #[test]
    fn border_width_override_never_authors_a_missing_border_role() {
        let mut app = widget_sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            slotted_widget_fill_tree(0),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let widget = styled_widget(&mut app);
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_border_widths([0.01; 4]),
        );
        app.update();

        let records = sdf_records(&app);
        assert_eq!(
            records[0].border_material,
            SdfPaintMaterial::NotAuthored,
            "a width override must not author a missing border role",
        );
        assert_eq!(
            records[0].border_widths.value,
            Vec4::ZERO,
            "an unauthored border keeps its zero widths",
        );
    }

    #[test]
    fn border_color_override_never_authors_a_missing_fill_role() {
        let mut app = widget_sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            slotted_widget_border_only_tree(),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        let records = sdf_records(&app);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].fill_material, SdfPaintMaterial::NotAuthored);

        let widget = styled_widget(&mut app);
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_border_color(SLOT_OVERRIDE_COLOR),
        );
        app.update();

        let records = sdf_records(&app);
        assert_eq!(
            records[0].fill_material,
            SdfPaintMaterial::NotAuthored,
            "an override must not author a missing fill role",
        );
        assert_eq!(
            border_row_color(&app, &records[0]),
            linear_color(SLOT_OVERRIDE_COLOR),
        );
    }

    #[test]
    fn material_override_rekeys_record_and_retires_the_emptied_batch() {
        let mut app = widget_sdf_pipeline_app();
        let blend_material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                ..Default::default()
            });
        spawn_sdf_panel(
            &mut app,
            slotted_widget_fill_tree(1),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        let entities_before = live_sdf_batch_entities(&mut app);
        assert_eq!(entities_before.len(), 2);
        let (origin_entity, peer_entity, peer_records) = {
            let store = app.world().resource::<SdfBatchStore>();
            let origin = store
                .batches()
                .find(|(key, _)| key.z_index == 0.into())
                .and_then(|(_, batch)| batch.entity)
                .expect("the widget fill batch should be spawned");
            let (_, peer_batch) = store
                .batches()
                .find(|(key, _)| key.z_index == 1.into())
                .expect("the peer fill batch should exist");
            let peer = peer_batch.entity.expect("peer batch should be spawned");
            (origin, peer, peer_batch.records().to_vec())
        };

        let widget = styled_widget(&mut app);
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_material(blend_material.clone()),
        );
        app.update();

        let store = app.world().resource::<SdfBatchStore>();
        assert_eq!(store.batches().count(), 2);
        let blend_entity = store
            .batches()
            .find(|(key, _)| key.pipeline_compatibility.alpha == BatchAlphaMode::Blend)
            .and_then(|(_, batch)| batch.entity)
            .expect("the re-keyed record should live in a spawned Blend batch");
        let blend_records: Vec<ResolvedSdfBatchRecord> = app
            .world()
            .resource::<SdfBatchStore>()
            .batches()
            .filter(|(key, _)| key.pipeline_compatibility.alpha == BatchAlphaMode::Blend)
            .flat_map(|(_, batch)| batch.records().iter().cloned())
            .collect();
        assert_eq!(blend_records.len(), 1);
        assert!(!entities_before.contains(&blend_entity));
        assert_ignores_picking(&app, blend_entity);
        assert!(
            app.world().get_entity(origin_entity).is_err(),
            "the emptied origin batch should be retired",
        );
        let peer_after = app
            .world()
            .resource::<SdfBatchStore>()
            .batches()
            .find(|(key, _)| key.z_index == 1.into())
            .map(|(_, batch)| (batch.entity, batch.records().to_vec()))
            .expect("the peer batch should survive the re-key");
        assert_eq!(peer_after, (Some(peer_entity), peer_records));

        // The stable slot resolves the destination batch for later overrides.
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default()
                .with_material(blend_material)
                .with_offset(OVERRIDE_OFFSET),
        );
        app.update();
        let moved_translation = app
            .world()
            .resource::<SdfBatchStore>()
            .batches()
            .filter(|(key, _)| key.pipeline_compatibility.alpha == BatchAlphaMode::Blend)
            .flat_map(|(_, batch)| batch.records().iter())
            .map(|record| record.local_transform.translation)
            .next()
            .expect("moved record should stay resolvable through its slot");
        assert_eq!(
            moved_translation,
            blend_records[0].local_transform.translation + OVERRIDE_OFFSET.extend(0.0),
        );

        clear_slot_override(&mut app, widget);
        app.update();
        let store = app.world().resource::<SdfBatchStore>();
        assert_eq!(store.batches().count(), 2);
        assert!(
            store
                .batches()
                .all(|(key, _)| key.pipeline_compatibility.alpha == BatchAlphaMode::Opaque),
        );
        assert!(
            app.world().get_entity(blend_entity).is_err(),
            "the emptied Blend batch should be retired",
        );
    }

    /// A world-panel vertical slider driving a visible SDF thumb, plus an
    /// unrelated peer fill. `SliderDirection::BottomToTop` exercises the
    /// render-frame Y inversion, and the `Mm` world panel applies a non-unit
    /// world scale, so a raw layout-point offset renders at the wrong place.
    fn vertical_thumb_slider_tree() -> LayoutTree {
        let slider = Slider::new(0.0..=1.0)
            .value(0.0)
            .direction(SliderDirection::BottomToTop);
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::overlay()
                .size(20.0, 30.0)
                .background(WIDGET_FILL_COLOR)
                .border(Border::all(1.0, IMAGE_BORDER_COLOR))
                .alignment(AlignX::Center, AlignY::Center)
                .widget("level", slider),
            |builder| {
                builder.with(
                    El::new()
                        .size(16.0, 8.0)
                        .background(SLOT_OVERRIDE_COLOR)
                        .id("thumb")
                        .slider_thumb(),
                    |_| {},
                );
            },
        );
        builder.with(
            El::new().size(20.0, 10.0).background(PEER_FILL_COLOR),
            |_| {},
        );
        builder.build()
    }

    #[test]
    fn slider_value_change_moves_only_the_thumb_sdf_record_by_the_converted_render_delta() {
        let mut app = widget_sdf_pipeline_app();
        let panel = spawn_sdf_panel(
            &mut app,
            vertical_thumb_slider_tree(),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        app.update();

        let widget = styled_widget(&mut app);
        let slots = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .cloned()
            .expect("slider widget should carry visual slots");
        let content = slots
            .content_box(VisualSlotId::SLIDER_ROOT)
            .expect("slider root content box");
        let thumb = slots
            .border_box(VisualSlotId::SLIDER_THUMB)
            .expect("thumb border box");
        let thumb_index = slots
            .element_index(VisualSlotId::SLIDER_THUMB)
            .expect("thumb element index");
        let root_index = slots
            .element_index(VisualSlotId::SLIDER_ROOT)
            .expect("root element index");
        assert!(
            thumb.height < content.height,
            "the thumb must be shorter than the content for a vertical travel",
        );

        // Derive the expected render offsets independently from the real panel
        // scale and the solved slot boxes — not from the production conversion
        // helper. A `BottomToTop` slider maps value 0 to the bottom (larger
        // layout Y) and value 1 to the top (smaller layout Y); the render frame
        // scales both axes and inverts the layout Y axis.
        let scale = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("panel role component")
            .points_to_world();
        assert!(
            scale.is_finite() && scale > 0.0 && (scale - 1.0).abs() > f32::EPSILON,
            "the world panel must apply a finite, non-unit, positive scale",
        );
        let authored_center = thumb.center().1;
        let bottom_center = thumb.height.mul_add(-0.5, content.y + content.height);
        let top_center = thumb.height.mul_add(0.5, content.y);
        let render_offset = |value: f32| {
            let desired_center = value.mul_add(top_center - bottom_center, bottom_center);
            Vec3::new(0.0, -(desired_center - authored_center) * scale, 0.0)
        };
        let expected_offset_at_end = render_offset(1.0);
        let expected_record_delta = render_offset(1.0) - render_offset(0.0);
        assert!(
            expected_record_delta.length() > 1e-6,
            "the value change must move the thumb a measurable amount",
        );

        let records_before = sdf_records(&app);
        let snapshot = widget_geometry_snapshot(&app, panel, widget);

        // Apply a different applied value; only the thumb presentation reacts.
        {
            let mut state = app
                .world_mut()
                .get_mut::<SliderState>(widget)
                .expect("slider widget should carry slider state");
            assert!(state.set_value(1.0).expect("value applies"));
        }
        app.update();

        let records_after = sdf_records(&app);
        assert_eq!(
            records_after.len(),
            records_before.len(),
            "the value change must not add or drop a retained record",
        );

        // Exactly one retained record moves, by the converted panel-local
        // render delta; every other record stays byte-for-byte unchanged.
        let mut moved = Vec::new();
        for (index, (before, after)) in records_before.iter().zip(&records_after).enumerate() {
            let delta = after.local_transform.translation - before.local_transform.translation;
            if delta.length() > 1e-6 {
                moved.push(index);
                assert!(
                    (delta - expected_record_delta).length() < 1e-6,
                    "the thumb record must shift by the converted render delta, \
                     not raw layout points or an uninverted Y",
                );
            } else {
                assert_eq!(
                    after, before,
                    "a non-thumb retained record must stay unchanged"
                );
            }
        }
        assert_eq!(moved.len(), 1, "only the thumb record may move");

        // The moved record is the thumb: its slot carries the converted render
        // offset in the override index, while the plain slider root carries no
        // override at all.
        let index = app.world().resource::<VisualOverrideIndex>();
        let indexed_thumb_offset = index
            .get(panel, thumb_index)
            .and_then(|value| value.offset)
            .expect("thumb slot should carry a render offset in the override index");
        assert!(
            (indexed_thumb_offset - expected_offset_at_end.truncate()).length() < 1e-6,
            "the indexed thumb offset must equal the independently converted render offset",
        );
        assert!(
            index.get(panel, root_index).is_none(),
            "a plain slider root writes no override",
        );

        // The value change moved no widget hit rectangle or panel layout output.
        assert_widget_geometry_unchanged(&app, panel, widget, &snapshot);
    }

    /// Widget image pipeline app plus one panel: a slotted image inside the
    /// `styled` button widget and an unslotted peer image, both sampling
    /// `image_a`.
    fn slotted_image_fixture() -> (App, Entity) {
        let mut app = image_sdf_pipeline_app();
        app.add_plugins(WidgetsPlugin);
        let image_a = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::default());
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::column().width(Sizing::GROW).height(Sizing::GROW),
            |builder| {
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .button("styled"),
                    |builder| {
                        builder.image(
                            El::new()
                                .width(Sizing::GROW)
                                .height(Sizing::GROW)
                                .visual_slot(TEST_SLOT),
                            image_a.clone(),
                            Color::WHITE,
                        );
                    },
                );
                builder.image(
                    El::new().width(Sizing::GROW).height(Sizing::GROW),
                    image_a.clone(),
                    Color::WHITE,
                );
            },
        );
        let panel = spawn_sdf_panel(&mut app, builder.build(), StandardMaterial::default());
        settle_sdf_pipeline(&mut app);
        (app, panel)
    }

    /// `ImageRenderRecord` storage buffer of the single live image batch.
    fn image_record_buffer(app: &App) -> Handle<ShaderBuffer> {
        let buffers: Vec<Handle<ShaderBuffer>> = app
            .world()
            .resource::<ImageBatchStore>()
            .batches()
            .filter_map(|(_, batch)| batch.gpu.as_ref().map(|gpu| gpu.records.clone()))
            .collect();
        assert_eq!(buffers.len(), 1, "expected exactly one image record buffer");
        buffers[0].clone()
    }

    #[test]
    fn tint_override_patches_only_the_referenced_image_record() {
        let (mut app, panel) = slotted_image_fixture();
        let records_before = image_records(&app);
        assert_eq!(records_before.len(), 2);
        assert_eq!(
            app.world().resource::<ImageBatchStore>().batches().count(),
            1,
        );

        let widget = styled_widget(&mut app);
        app.update();
        let snapshot = widget_geometry_snapshot(&app, panel, widget);
        let record_buffer = image_record_buffer(&app);
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_tint(SLOT_OVERRIDE_COLOR),
        );
        app.update();

        let records = image_records(&app);
        assert_eq!(records[0].tint, linear_color(SLOT_OVERRIDE_COLOR));
        assert_eq!(records[1], records_before[1], "peer record untouched");
        assert_eq!(
            app.world().resource::<ImageBatchStore>().batches().count(),
            1,
        );
        assert!(
            app.world()
                .resource::<RetainedBatchBufferUploads>()
                .contains(&record_buffer),
            "the changed image record should stage a direct same-frame GPU write",
        );
        assert_widget_geometry_unchanged(&app, panel, widget, &snapshot);

        // Author the identical override a second time: the route rebuilds an
        // equal record, so no retained record changes and the record buffer
        // does not re-upload.
        let records_after = image_records(&app);
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_tint(SLOT_OVERRIDE_COLOR),
        );
        app.update();
        assert_eq!(image_records(&app), records_after);
        assert!(
            !app.world()
                .resource::<RetainedBatchBufferUploads>()
                .contains(&record_buffer),
            "a repeated identical override should stage no GPU write",
        );
        assert_widget_geometry_unchanged(&app, panel, widget, &snapshot);
    }

    #[test]
    fn texture_override_moves_image_record_to_the_destination_batch() {
        let (mut app, panel) = slotted_image_fixture();
        let image_b = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::default());
        let records_before = image_records(&app);
        let origin_entity = app
            .world()
            .resource::<ImageBatchStore>()
            .batches()
            .next()
            .and_then(|(_, batch)| batch.entity)
            .expect("settled image batch should be spawned");

        // A texture override changes the `ImageBatchKey` and moves the record.
        let widget = styled_widget(&mut app);
        let snapshot = widget_geometry_snapshot(&app, panel, widget);
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default().with_texture(image_b.clone()),
        );
        app.update();
        assert_widget_geometry_unchanged(&app, panel, widget, &snapshot);
        let store = app.world().resource::<ImageBatchStore>();
        assert_eq!(store.batches().count(), 2);
        let destination_entity = store
            .batches()
            .find(|(key, _)| key.texture == image_b)
            .and_then(|(_, batch)| batch.entity)
            .expect("the moved record should live in a spawned destination batch");
        assert_ne!(destination_entity, origin_entity);
        assert_ignores_picking(&app, destination_entity);

        // The stable slot resolves the destination batch for later overrides.
        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride::default()
                .with_texture(image_b.clone())
                .with_tint(SLOT_OVERRIDE_COLOR),
        );
        app.update();
        let destination_tints: Vec<Vec4> = app
            .world()
            .resource::<ImageBatchStore>()
            .batches()
            .filter(|(key, _)| key.texture == image_b)
            .flat_map(|(_, batch)| batch.records().iter())
            .map(|record| record.tint)
            .collect();
        assert_eq!(destination_tints, vec![linear_color(SLOT_OVERRIDE_COLOR)]);

        clear_slot_override(&mut app, widget);
        app.update();
        assert_eq!(
            app.world().resource::<ImageBatchStore>().batches().count(),
            1,
        );
        assert!(
            app.world().get_entity(destination_entity).is_err(),
            "the emptied destination batch should be retired",
        );
        assert_eq!(image_records(&app), records_before);
    }
}
