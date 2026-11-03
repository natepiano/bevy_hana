//! Batched SDF fill route backed by the frame material table.
//!
//! Builds typed SDF fill/border records and their material-table rows in one
//! frame-local pass. Production panel backgrounds, borders, and divider
//! rectangles render through visible `DiegeticSdfFillBatch` entities.

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;

use bevy::asset::Asset;
use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::visibility::VisibilitySystems;
use bevy::ecs::system::SystemParam;
use bevy::image::Image;
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
use bevy::prelude::Changed;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Handle;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Mat4;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::Or;
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
use bevy::render::render_resource::ShaderType;
use bevy::render::render_resource::SpecializedMeshPipelineError;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use bevy::transform::TransformSystems;
use bytemuck::Pod;
use bytemuck::Zeroable;
use hana_kana::ToU32;
use hana_kana::ToUsize;

use super::AncestorVisibility;
use super::BatchAlphaMode;
use super::BatchRenderLayers;
use super::CommandIndex;
use super::Dirty;
use super::ElementIndex;
use super::TooltipDepthPriority;
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
use super::constants::OIT_PAINT_STEP;
use super::constants::SDF_AA_PADDING;
#[cfg(test)]
use super::draw_order;
use super::draw_order::DrawCommandDepth;
use super::draw_order::DrawOrderIndex;
use super::draw_order::DrawZIndexRank;
use super::material;
use super::material::PanelDisplayTarget;
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
use super::material_table::MaterialTableBufferPrepared;
use super::material_table::RetainedBatchBufferUploads;
#[cfg(test)]
use super::material_table::SdfDriverRunOrder;
use super::material_table::SdfPaintMaterial;
use super::panel_geometry::ResolvedSdfMaterial;
use super::panel_geometry::ResolvedSdfSurface;
use super::panel_geometry::ResolvedSdfSurfaceRegistry;
use super::panel_geometry::SdfRecordRole;
use super::panel_geometry::SdfSourcePart;
use super::panel_geometry::StoredResolvedSdfSurface;
use super::panel_geometry::SurfaceEvictionCause;
use crate::DrawZIndex;
use crate::cascade::Resolved;
use crate::cascade::SdfMaterial;
use crate::constants::SDF_PANEL_SHADER_HANDLE;
use crate::layout::DrawBatchFamily;
use crate::layout::Lighting;
use crate::layout::ShadowCasting;
use crate::layout::Sidedness;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::DiegeticPanel;
use crate::panel::DiegeticPerfStats;
use crate::panel::DroppedSdfSurfaces;
use crate::panel::FrameWork;
use crate::render;
use crate::screen_space::ScreenPixelEdgeSnap;
use crate::widgets::SliderDirection;
use crate::widgets::TooltipFor;
use crate::widgets::TooltipPaintInsideBorder;
use crate::widgets::VisualOverrideIndex;
use crate::widgets::VisualSlotOverride;
use crate::widgets::WidgetOf;
use crate::widgets::WidgetSpec;

/// Extra `clip_depth_nudge` layer-units pushing an opaque/alpha-mask SDF fill away
/// from the camera so coplanar panel text, drawn at its true depth, wins the
/// depth test instead of z-fighting the fill. Only the depth-buffer regime
/// (`Opaque`/`Mask`) applies it; transparent and OIT fills order through
/// `ScreenDepthBias`/`OitPaintOrder` and get zero here. Starts far below one
/// layer-unit; raise by powers of ten until text shows without the fill
/// visibly floating off the surface.
const OPAQUE_FILL_DEPTH_PUSH_LAYERS: f32 = 1.0;
const SDF_TOOLTIP_DEPTH_PRIORITY_FLAG: u32 = 1;
const SDF_PAINT_INSIDE_BORDER_FLAG: u32 = 1 << 1;
const SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG: u32 = 1 << 2;
const SDF_TOOLTIP_KEEP_SNAPPED_EDGES_INSIDE_FLAG: u32 = 1 << 3;
const SDF_CLIP_DEPTH_NUDGE_PER_LAYER: f32 = 0.000_000_02;
/// Leave one SDF clip-nudge unit below the OIT opaque-occlusion tolerance.
const OPAQUE_FILL_MAX_CLIP_NUDGE_LAYERS: f32 =
    OIT_PAINT_STEP / SDF_CLIP_DEPTH_NUDGE_PER_LAYER - 1.0;
/// Largest f32 ulp of a reverse-Z depth value, which lies in `[0, 1]`.
const DEPTH_ULP_BELOW_ONE: f32 = f32::EPSILON / 2.0;
/// Whole depth ulps a tooltip shadow sits behind its opaque body without OIT,
/// so the body's depth test hides the overlap on every GPU at every depth.
/// Each step is `DEPTH_ULP_BELOW_ONE`, never less than one ulp anywhere in the
/// depth range, and the whole margin stays far inside the tooltip's
/// `TOOLTIP_NDC_DEPTH_PRIORITY` lift.
const TOOLTIP_SHADOW_BEHIND_BODY_DEPTH_ULPS: f32 = 16.0;
/// `TOOLTIP_SHADOW_BEHIND_BODY_DEPTH_ULPS` in SDF clip-nudge layer units.
const TOOLTIP_SHADOW_BEHIND_BODY_LAYERS: f32 =
    TOOLTIP_SHADOW_BEHIND_BODY_DEPTH_ULPS * DEPTH_ULP_BELOW_ONE / SDF_CLIP_DEPTH_NUDGE_PER_LAYER;

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
    /// Tooltip body shadow, keyed apart from the body's own fill.
    TooltipShadow,
}

impl SdfMaterialRole {
    /// Returns the source role that keys `surface`'s fill row.
    const fn fill_of(surface: &ResolvedSdfSurface<'_>) -> Self {
        match surface.record_role {
            SdfRecordRole::TooltipShadow(_) => Self::TooltipShadow,
            SdfRecordRole::Whole | SdfRecordRole::Fill | SdfRecordRole::Border => Self::Fill,
        }
    }
}

/// Material source identity for one SDF fill or border role.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct SdfMaterialSourceKey {
    /// Panel entity that owns this role.
    pub panel:       Entity,
    /// Source element in the panel's layout tree.
    pub element:     ElementIndex,
    /// Element surface or child divider that owns this material role.
    pub source_part: SdfSourcePart,
    /// Role within the resolved SDF surface.
    pub role:        SdfMaterialRole,
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
    pub panel:       Entity,
    /// Source element in the panel's layout tree.
    pub element:     ElementIndex,
    /// Element surface or child divider that owns this record.
    pub source_part: SdfSourcePart,
    /// Role produced when clipping separates a fill from its border.
    pub role:        SdfRecordRole,
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

/// Shader-local Y-down SDF clip rectangle after AA padding expansion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LocalClipRect {
    /// `[left, top, right, bottom]` clip rectangle.
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

/// Fill and border role-presence bits read by `sdf_panel.wesl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, ShaderType, Pod, Zeroable)]
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

/// Ordinal of one maximal run of compatible SDF records in its panel's draw
/// order.
///
/// Example: a panel draws `background`, `badge`, `border`, where `badge` needs
/// another pipeline. `background` and `border` must not merge into one batch,
/// which would draw `border` before `badge`; they take runs 0 and 2, and
/// `SdfBatchKey::sort_depth_bias` sorts each run after the panel's earlier
/// ones.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(super) struct ContiguousDrawnRun {
    /// Number of the panel's runs drawn before this one.
    pub value: u32,
}

/// Key for one SDF fill batch and one `SdfBatchResources` entry.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct SdfBatchKey {
    /// Camera transform the panel reaches after fragment output.
    pub display_target:         PanelDisplayTarget,
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
    /// Where the batch sorts among non-OIT `Transparent3d` draws.
    pub transparent_sort:       SdfTransparentSort,
    /// Which panels' records the batch may hold.
    pub batch_sharing:          SdfBatchSharing,
}

impl SdfBatchKey {
    /// What this batch's fragments paint. Only tooltip shadow records sort
    /// over a host, so the sort names a shadow batch.
    const fn fragment_paint(&self) -> SdfFragmentPaint {
        match self.transparent_sort {
            SdfTransparentSort::PanelRank => SdfFragmentPaint::Surface,
            SdfTransparentSort::OverHost(_) => SdfFragmentPaint::TooltipShadow,
        }
    }

    /// Lighting path compiled for this batch's homogeneous records.
    const fn fragment_lighting(&self) -> SdfFragmentLighting {
        if matches!(self.fragment_paint(), SdfFragmentPaint::TooltipShadow)
            || self.pipeline_compatibility.unlit
        {
            SdfFragmentLighting::UnlitOnly
        } else {
            SdfFragmentLighting::Pbr
        }
    }

    /// Non-OIT `Transparent3d` sort bias of this batch's material.
    ///
    /// Batches of one panel can share a sort base and a centroid depth, so the
    /// run ordinal lifts each run past the panel's earlier runs; otherwise
    /// entity order would decide which draws last.
    fn sort_depth_bias(&self) -> f32 {
        let base = match self.transparent_sort {
            SdfTransparentSort::PanelRank => self.z_index_rank.screen_depth_bias(),
            SdfTransparentSort::OverHost(host_rank) => host_rank.screen_tooltip_shadow_depth_bias(),
        };
        base.after_earlier_runs(self.contiguous_drawn_run.value)
            .get()
    }
}

/// Where an SDF batch sorts among a camera's non-OIT `Transparent3d` draws.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum SdfTransparentSort {
    /// By its z-index rank, among the draws of its own panel.
    PanelRank,
    /// Just past the text of the deepest rank its tooltip's host panel draws
    /// at. A tooltip shadow blends over the host draws under it, so it
    /// composites after them, and before a surface nearer the camera than that
    /// band; its depth still sits behind its own opaque body.
    OverHost(DrawZIndexRank),
}

/// Which panels an SDF record may share a batch with.
///
/// Records inside one blended batch draw in panel-local order, which cannot
/// place a tooltip over its host, so a tooltip's records draw only with each
/// other and its own batch sorts after the host's.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum SdfBatchSharing {
    /// Shares a batch with any panel's records.
    AnyPanel,
    /// Draws only with the records of this tooltip panel.
    TooltipOnly(Entity),
}

/// Per-panel facts that route one resolved surface into its batch.
#[derive(Clone, Copy, Debug)]
pub(super) struct SdfRecordRoute {
    /// Camera transform the panel reaches after fragment output.
    pub display_target:   PanelDisplayTarget,
    /// Where the record sorts among non-OIT `Transparent3d` draws.
    pub transparent_sort: SdfTransparentSort,
    /// Which panels the record may share a batch with.
    pub batch_sharing:    SdfBatchSharing,
}

/// Reads the panel a tooltip describes a widget on, which its shadow falls on.
#[derive(SystemParam)]
struct TooltipHosts<'w, 's> {
    tooltips: Query<'w, 's, &'static TooltipFor>,
    widgets:  Query<'w, 's, &'static WidgetOf>,
    panels:   Query<'w, 's, &'static ComputedDiegeticPanel>,
}

impl TooltipHosts<'_, '_> {
    /// Where `surface` sorts among non-OIT `Transparent3d` draws.
    ///
    /// A tooltip shadow sorts over its host panel's deepest rank; a tooltip
    /// that describes no panel widget has no host draws, so its shadow sorts
    /// over the first rank.
    fn transparent_sort(&self, surface: &ResolvedSdfSurface<'_>) -> SdfTransparentSort {
        match surface.record_role {
            SdfRecordRole::TooltipShadow(_) => SdfTransparentSort::OverHost(
                self.tooltips
                    .get(surface.panel_entity)
                    .and_then(|tooltip| self.widgets.get(tooltip.target()))
                    .and_then(|widget| self.panels.get(widget.panel()))
                    .map_or_else(
                        |_| DrawZIndexRank::default(),
                        |host| host.draw_order().deepest_z_index_rank(),
                    ),
            ),
            SdfRecordRole::Whole | SdfRecordRole::Fill | SdfRecordRole::Border => {
                SdfTransparentSort::PanelRank
            },
        }
    }

    /// Which panels `surface` may share a batch with: only its own panel when
    /// that panel is a tooltip, any panel otherwise.
    fn batch_sharing(&self, surface: &ResolvedSdfSurface<'_>) -> SdfBatchSharing {
        if self.tooltips.contains(surface.panel_entity) {
            SdfBatchSharing::TooltipOnly(surface.panel_entity)
        } else {
            SdfBatchSharing::AnyPanel
        }
    }
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

/// World-space physical plane shared by a panel's coplanar SDF surfaces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WorldPanelPlane {
    pub(super) coefficients: Vec4,
}

impl WorldPanelPlane {
    pub(super) fn from_panel(panel: &GlobalTransform, local_depth: f32) -> Self {
        Self::from_matrix(panel.to_matrix(), local_depth)
    }

    pub(super) fn from_matrix(transform: Mat4, local_depth: f32) -> Self {
        let normal = transform
            .x_axis
            .truncate()
            .cross(transform.y_axis.truncate());
        let origin = transform.transform_point3(Vec3::new(0.0, 0.0, local_depth));
        Self {
            coefficients: normal.extend(-normal.dot(origin)),
        }
    }
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
    /// Common physical plane, independent of this quad's position and extent.
    pub panel_plane:      WorldPanelPlane,
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
    /// Normalized local paint order used only at equal physical OIT depth.
    pub oit_paint_order:  f32,
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
        route: SdfRecordRoute,
    ) -> Self {
        let pipeline_compatibility = sdf_record_pipeline_compatibility(surface, &materials);
        let fill_source = SdfMaterialSourceKey {
            panel:       surface.panel_entity,
            element:     surface.element_index,
            source_part: surface.source_part,
            role:        SdfMaterialRole::fill_of(surface),
        };
        let border_source = SdfMaterialSourceKey {
            panel:       surface.panel_entity,
            element:     surface.element_index,
            source_part: surface.source_part,
            role:        SdfMaterialRole::Border,
        };
        let batch_key = SdfBatchKey {
            display_target: route.display_target,
            z_index: surface.draw_depth.z_index(),
            z_index_rank: surface.draw_depth.z_index_rank(),
            batch_family: DrawBatchFamily::SdfSurface,
            layers: BatchRenderLayers(surface.render_layers.clone()),
            shadow: surface.shadow_casting.into(),
            contiguous_drawn_run,
            pipeline_compatibility,
            resource_compatibility: materials.resource_compatibility.clone(),
            transparent_sort: route.transparent_sort,
            batch_sharing: route.batch_sharing,
        };
        let paint_mask = SdfPaintMask::from_materials(materials.fill, materials.border);
        Self {
            record_key: SdfRecordKey {
                panel:       surface.panel_entity,
                element:     surface.element_index,
                source_part: surface.source_part,
                role:        surface.record_role,
            },
            draw_order: surface.command_index,
            fill_source,
            border_source,
            draw_depth: surface.draw_depth,
            batch_key,
            local_transform: surface.local_transform,
            transform: surface.local_transform.to_matrix(),
            panel_plane: WorldPanelPlane::from_panel(
                &GlobalTransform::IDENTITY,
                surface.local_transform.translation.z,
            ),
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
            oit_paint_order: surface.draw_depth.oit_paint_order().get(),
            flags: 0,
        }
    }

    /// Rewrites `SdfRenderRecord::transform` from the panel's propagated transform.
    fn update_world_transform(&mut self, panel_transform: &GlobalTransform) {
        self.transform = panel_transform.to_matrix() * self.local_transform.to_matrix();
        self.panel_plane =
            WorldPanelPlane::from_panel(panel_transform, self.local_transform.translation.z);
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
        && (matches!(surface.record_role, SdfRecordRole::Border) || surface.clip_rect_limits_mesh())
}

/// GPU mirror for one batched SDF fill/border surface.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, ShaderType, Pod, Zeroable)]
struct SdfRenderRecord {
    /// World-space transform used by the vertex shader.
    transform:        Mat4,
    /// Common world plane used to reconstruct coplanar OIT fragment depths.
    panel_plane:      Vec4,
    /// Rounded-rectangle SDF half-size in local record space.
    half_size:        Vec2,
    /// Quad half-size including AA padding in local record space.
    mesh_half_size:   Vec2,
    /// Per-corner radii `[top_left, top_right, bottom_right, bottom_left]`.
    corner_radii:     Vec4,
    /// Per-side widths `[top, right, bottom, left]`.
    border_widths:    Vec4,
    /// Shader-local Y-down clip rectangle `[left, top, right, bottom]`.
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
    /// Normalized local paint order used only at equal physical OIT depth.
    oit_paint_order:  f32,
    /// Tooltip shadow `(blur support, downward offset)` in local record space.
    tooltip_shadow:   Vec2,
}

impl SdfRenderRecord {
    /// Converts the CPU retained record to the GPU storage-buffer mirror.
    #[must_use]
    fn from_resolved(
        record: &ResolvedSdfBatchRecord,
        first_draw_order_index: DrawOrderIndex,
    ) -> Self {
        let tooltip_shadow = match record.record_key.role {
            SdfRecordRole::TooltipShadow(shadow) => {
                Vec2::new(shadow.blur_support, shadow.downward_offset)
            },
            SdfRecordRole::Whole | SdfRecordRole::Fill | SdfRecordRole::Border => Vec2::ZERO,
        };
        Self {
            transform: record.transform,
            panel_plane: record.panel_plane.coefficients,
            half_size: record.half_size.value,
            mesh_half_size: record.mesh_half_size.value,
            corner_radii: record.corner_radii.value,
            border_widths: record.border_widths.value,
            clip_rect: record.clip_rect.value,
            fill_material: record.fill_material.to_gpu(),
            border_material: record.border_material.to_gpu(),
            paint_mask: record.paint_mask,
            flags: record.flags,
            clip_depth_nudge: record.clip_depth_nudge
                - first_draw_order_index.clip_depth_nudge().get(),
            oit_paint_order: record.oit_paint_order,
            tooltip_shadow,
        }
    }

    /// Capacity-tail record that collapses before any material-table read.
    #[must_use]
    const fn padded() -> Self {
        Self {
            transform:        Mat4::ZERO,
            panel_plane:      Vec4::ZERO,
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
            oit_paint_order:  0.0,
            tooltip_shadow:   Vec2::ZERO,
        }
    }
}

/// Placeholder SDF mesh table row bound at `SDF_MESH_BINDING`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType, Pod, Zeroable)]
struct SdfMeshRecord {
    /// Reserved data lane for future mesh-level SDF batch metadata.
    reserved: UVec4,
}

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
enum SdfBatchAllocation {
    /// No render entity or GPU resources exist yet.
    #[default]
    Unallocated,
    /// The render entity and all GPU resources exist together.
    Allocated {
        entity:    Entity,
        resources: SdfBatchResources,
    },
}

impl SdfBatchAllocation {
    const fn entity(&self) -> Option<Entity> {
        match self {
            Self::Unallocated => None,
            Self::Allocated { entity, .. } => Some(*entity),
        }
    }

    const fn resources(&self) -> Option<&SdfBatchResources> {
        match self {
            Self::Unallocated => None,
            Self::Allocated { resources, .. } => Some(resources),
        }
    }

    const fn resources_mut(&mut self) -> Option<&mut SdfBatchResources> {
        match self {
            Self::Unallocated => None,
            Self::Allocated { resources, .. } => Some(resources),
        }
    }
}

/// CPU record set and allocation state for one `SdfBatchKey`.
#[derive(Debug, Default)]
pub(super) struct SdfBatch {
    /// Render entity and GPU resources, allocated atomically.
    allocation:             SdfBatchAllocation,
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
    /// `SdfRenderRecord::oit_paint_order` stays panel-absolute.
    first_draw_order_index: DrawOrderIndex,
    records:                Vec<ResolvedSdfBatchRecord>,
}

impl SdfBatch {
    #[cfg(test)]
    const fn resources(&self) -> Option<&SdfBatchResources> { self.allocation.resources() }

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
            record.panel_plane = self.records[position].panel_plane;
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

    fn entity(&self) -> Option<Entity> { self.allocation.entity() }
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
pub(super) struct SdfBatchStore {
    batches:       BatchStore<SdfBatchKey, SdfBatch>,
    panel_members: HashMap<Entity, Vec<SdfRecordKey>>,
}

impl SdfBatchStore {
    /// Inserts or moves one retained SDF record.
    fn upsert_record(&mut self, record: ResolvedSdfBatchRecord) {
        let record_key = record.record_key;
        let batch_key = record.batch_key.clone();
        self.batches.upsert(batch_key, record_key, record);
    }

    /// Retires only this panel's records that its current route did not publish.
    fn retain_panel_records(&mut self, panel: Entity, active: &HashSet<SdfRecordKey>) {
        if self.panel_members.is_empty() {
            // With no prior panel membership, every routed record belongs to
            // this first panel, so the store-wide retain is both correct and
            // avoids constructing a stale-member list.
            self.batches.retain(active);
            if !active.is_empty() {
                self.panel_members
                    .insert(panel, active.iter().copied().collect());
            }
            return;
        }
        if let Some(members) = self.panel_members.get(&panel) {
            for &stale in members.iter().filter(|key| !active.contains(key)) {
                self.batches.remove(stale);
            }
        }
        if active.is_empty() {
            self.panel_members.remove(&panel);
        } else {
            self.panel_members
                .insert(panel, active.iter().copied().collect());
        }
    }

    /// All SDF batches.
    fn batches(&self) -> impl Iterator<Item = (&SdfBatchKey, &SdfBatch)> { self.batches.batches() }

    /// All SDF batches, mutable.
    fn batches_mut(&mut self) -> impl Iterator<Item = (&SdfBatchKey, &mut SdfBatch)> {
        self.batches.batches_mut()
    }

    /// One SDF batch by key, mutable.
    fn get_mut(&mut self, key: &SdfBatchKey) -> Option<&mut SdfBatch> { self.batches.get_mut(key) }

    /// Drops empty batch entries, returning their entities for despawn.
    fn take_empty_batches(&mut self) -> Vec<Entity> { self.batches.take_empty_batches() }
}

struct SdfMemberFamily;

impl MemberFamily for SdfMemberFamily {
    type Key = SdfBatchKey;
    type Batch = SdfBatch;
    type Store = SdfBatchStore;
    type Marker = DiegeticSdfFillBatch;

    fn store_mut(store: &mut Self::Store) -> &mut BatchStore<Self::Key, Self::Batch> {
        &mut store.batches
    }
}

/// How an SDF batch pipeline sources its per-quad geometry.
#[derive(Clone, Copy, Eq, Hash, PartialEq, Debug)]
enum SdfPipelineMode {
    /// Pulls per-quad geometry from the record storage buffer in the vertex
    /// stage (`sdf_panel.wesl`); the only production mode.
    VertexPulled,
    /// Reads geometry from mesh vertex attributes instead of pulling it.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "non-pulled geometry mode is not produced yet")
    )]
    MeshAttributes,
}

/// What an SDF batch's fragments paint, which picks the one fragment path its
/// pipeline compiles.
#[derive(Clone, Copy, Eq, Hash, PartialEq, Debug)]
enum SdfFragmentPaint {
    /// Panel and element fills and borders.
    Surface,
    /// A tooltip body's blurred drop shadow, behind `SDF_TOOLTIP_SHADOW`.
    TooltipShadow,
}

/// Lighting code that an SDF batch's fragment records can execute.
#[derive(Clone, Copy, Eq, Hash, PartialEq, Debug)]
enum SdfFragmentLighting {
    /// Every record uses the shader's unlit color path.
    UnlitOnly,
    /// Records use Bevy's PBR lighting path.
    Pbr,
}

/// SDF material extension over `StandardMaterial`.
#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
#[bind_group_data(SdfExtensionKey)]
pub(crate) struct SdfExtension {
    /// SDF render records read by vertex and fragment stages.
    #[storage(107, read_only, visibility(vertex, fragment))]
    records:                         Handle<ShaderBuffer>,
    /// Shared `MaterialSlotValues` table read by SDF fragments.
    #[storage(106, read_only, visibility(fragment))]
    material_table:                  Handle<ShaderBuffer>,
    /// Reserved SDF mesh metadata table, present for binding-layout parity.
    #[storage(108, read_only, visibility(vertex))]
    mesh_records:                    Handle<ShaderBuffer>,
    /// Inverse display transform for unlit fills and borders.
    #[texture(109, dimension = "3d", visibility(fragment))]
    #[sampler(110, visibility(fragment))]
    display_table:                   Handle<Image>,
    /// Device pixels in one logical pixel, used to floor thin SDF strokes.
    #[uniform(111)]
    device_pixels_per_logical_pixel: f32,
    /// Geometry-sourcing mode for `sdf_panel.wesl`.
    pipeline_mode:                   SdfPipelineMode,
    /// Hardware depth-bias integer of the batch's z-index rank. The base
    /// material's `depth_bias` carries the `Transparent3d` sort bias, which a
    /// tooltip shadow raises past its host's draws without moving its depth.
    hardware_depth_bias:             i32,
    /// Fragment path `sdf_panel.wesl` compiles for this batch.
    fragment_paint:                  SdfFragmentPaint,
    /// Lighting path `sdf_panel.wesl` compiles for this batch.
    fragment_lighting:               SdfFragmentLighting,
}

/// Pipeline-specialization key for `SdfExtension`.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(crate) struct SdfExtensionKey {
    /// Mirror of `SdfExtension::pipeline_mode`.
    pipeline_mode:       SdfPipelineMode,
    /// Mirror of `SdfExtension::hardware_depth_bias`.
    hardware_depth_bias: i32,
    /// Mirror of `SdfExtension::fragment_paint`.
    fragment_paint:      SdfFragmentPaint,
    /// Mirror of `SdfExtension::fragment_lighting`.
    fragment_lighting:   SdfFragmentLighting,
}

impl From<&SdfExtension> for SdfExtensionKey {
    fn from(extension: &SdfExtension) -> Self {
        Self {
            pipeline_mode:       extension.pipeline_mode,
            hardware_depth_bias: extension.hardware_depth_bias,
            fragment_paint:      extension.fragment_paint,
            fragment_lighting:   extension.fragment_lighting,
        }
    }
}

impl MaterialExtension for SdfExtension {
    fn vertex_shader() -> ShaderRef { SDF_PANEL_SHADER_HANDLE.into() }

    fn fragment_shader() -> ShaderRef { SDF_PANEL_SHADER_HANDLE.into() }

    fn prepass_vertex_shader() -> ShaderRef { SDF_PANEL_SHADER_HANDLE.into() }

    fn prepass_fragment_shader() -> ShaderRef { SDF_PANEL_SHADER_HANDLE.into() }

    fn deferred_vertex_shader() -> ShaderRef { SDF_PANEL_SHADER_HANDLE.into() }

    fn deferred_fragment_shader() -> ShaderRef { SDF_PANEL_SHADER_HANDLE.into() }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if descriptor.fragment.as_ref().is_some_and(|fragment| {
            fragment
                .shader_defs
                .contains(&"MATERIAL_OIT_ENABLED".into())
        }) {
            descriptor
                .vertex
                .shader_defs
                .push("MATERIAL_OIT_ENABLED".into());
        }
        add_sdf_stripped_material_group_def(descriptor, key.bind_group_data.pipeline_mode);
        add_sdf_fragment_paint_def(descriptor, key.bind_group_data.fragment_paint);
        add_sdf_fragment_lighting_def(descriptor, key.bind_group_data.fragment_lighting);
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.bias.constant = key.bind_group_data.hardware_depth_bias;
        }
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

/// Compiles the tooltip shadow's fragment path into a shadow batch's pipeline
/// only, so no surface pipeline carries it.
fn add_sdf_fragment_paint_def(
    descriptor: &mut RenderPipelineDescriptor,
    fragment_paint: SdfFragmentPaint,
) {
    if matches!(fragment_paint, SdfFragmentPaint::TooltipShadow)
        && let Some(fragment) = descriptor.fragment.as_mut()
    {
        fragment.shader_defs.push("SDF_TOOLTIP_SHADOW".into());
    }
}

/// Compiles Bevy's PBR lighting path only when a batch can execute it.
fn add_sdf_fragment_lighting_def(
    descriptor: &mut RenderPipelineDescriptor,
    fragment_lighting: SdfFragmentLighting,
) {
    if matches!(fragment_lighting, SdfFragmentLighting::Pbr)
        && let Some(fragment) = descriptor.fragment.as_mut()
    {
        fragment.shader_defs.push("SDF_PBR_LIGHTING".into());
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
    pub key:           SdfBatchKey,
    /// SDF render-record buffer.
    pub records:       Handle<ShaderBuffer>,
    /// Reserved SDF mesh-record buffer.
    pub mesh_records:  Handle<ShaderBuffer>,
    pub display_table: Handle<Image>,
}

/// Builds the render material for one SDF batch.
#[must_use]
fn sdf_batch_material(input: SdfBatchMaterialInput) -> SdfExtendedMaterial {
    let SdfBatchMaterialInput {
        key,
        records,
        mesh_records,
        display_table,
    } = input;
    let mut base = batch_key::apply_resource_compatibility_to_standard_material(
        &material::default_panel_material(),
        &key.resource_compatibility,
    );
    base.alpha_mode = sdf_batch_alpha_mode(key.pipeline_compatibility.alpha, key.shadow);
    base.double_sided = key.pipeline_compatibility.double_sided;
    base.cull_mode = key.pipeline_compatibility.cull_mode.into();
    base.unlit = key.pipeline_compatibility.unlit;
    base.fog_enabled = key.pipeline_compatibility.fog_enabled;
    base.opaque_render_method = key.pipeline_compatibility.opaque_render_method.into();
    base.deferred_lighting_pass_id = key.pipeline_compatibility.deferred_lighting_pass_id;
    base.depth_bias = key.sort_depth_bias();
    ExtendedMaterial {
        base,
        extension: SdfExtension {
            records,
            material_table: Handle::default(),
            mesh_records,
            display_table,
            device_pixels_per_logical_pixel: 1.0,
            pipeline_mode: SdfPipelineMode::VertexPulled,
            hardware_depth_bias: key.z_index_rank.hardware_depth_bias(),
            fragment_paint: key.fragment_paint(),
            fragment_lighting: key.fragment_lighting(),
        },
    }
}

/// Updates the display scale used to floor thin SDF strokes in logical pixels.
pub(super) const fn set_sdf_material_logical_pixel_scale(
    material: &mut SdfExtendedMaterial,
    device_pixels_per_logical_pixel: f32,
) {
    material.extension.device_pixels_per_logical_pixel = device_pixels_per_logical_pixel;
}

/// SDF batch alpha-mode rule for authored source materials.
///
/// Table-row base-color alpha controls normal transparent composition inside
/// `sdf_panel.wesl`. In prepass/shadow pipelines, `fill_alpha >
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
/// OIT fills order through `ScreenDepthBias`/`OitPaintOrder` instead, so they
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
        SdfMaterialRole::fill_of(surface),
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
        SdfMaterialRole::Fill | SdfMaterialRole::TooltipShadow => &surface.fill_material,
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
            source_part: surface.source_part,
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

/// Assigns each record the ordinal of its contiguous run in its own panel's
/// draw order.
///
/// `DrawOrderIndex` orders one panel's commands only, so runs are counted per
/// panel: another panel's records, such as a shown tooltip's, never split a
/// panel's run.
fn assign_contiguous_runs(
    mut records: Vec<(
        SdfRecordMaterialSlots,
        &ResolvedSdfSurface<'_>,
        SdfRecordRoute,
    )>,
) -> Vec<ResolvedSdfBatchRecord> {
    records.sort_by(|(_, left, _), (_, right, _)| {
        left.draw_depth
            .draw_order_index()
            .cmp(&right.draw_depth.draw_order_index())
            .then(left.command_index.cmp(&right.command_index))
    });

    let mut open_runs = HashMap::<Entity, OpenSdfRun>::new();
    records
        .into_iter()
        .map(|(materials, surface, route)| {
            let compatibility = SdfRunCompatibility::from_surface(surface, &materials, route);
            let run = match open_runs.entry(surface.panel_entity) {
                Entry::Occupied(open) => open.into_mut().extend_or_follow(compatibility),
                Entry::Vacant(first) => {
                    first
                        .insert(OpenSdfRun {
                            compatibility,
                            run: ContiguousDrawnRun::default(),
                        })
                        .run
                },
            };
            ResolvedSdfBatchRecord::from_resolved(surface, materials, run, route)
        })
        .collect()
}

/// The run a panel's latest record joined.
struct OpenSdfRun {
    compatibility: SdfRunCompatibility,
    run:           ContiguousDrawnRun,
}

impl OpenSdfRun {
    /// Returns the run of the panel's next record: this run when the record is
    /// compatible with it, the panel's next run otherwise.
    fn extend_or_follow(&mut self, compatibility: SdfRunCompatibility) -> ContiguousDrawnRun {
        if self.compatibility != compatibility {
            self.compatibility = compatibility;
            self.run.value = self.run.value.saturating_add(1);
        }
        self.run
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SdfRunCompatibility {
    z_index:                DrawZIndex,
    z_index_rank:           DrawZIndexRank,
    layers:                 BatchRenderLayers,
    shadow:                 VisualShadow,
    pipeline_compatibility: PipelineCompatibility,
    resource_compatibility: ResourceCompatibility,
    transparent_sort:       SdfTransparentSort,
}

impl SdfRunCompatibility {
    fn from_surface(
        surface: &ResolvedSdfSurface<'_>,
        materials: &SdfRecordMaterialSlots,
        route: SdfRecordRoute,
    ) -> Self {
        Self {
            z_index:                surface.draw_depth.z_index(),
            z_index_rank:           surface.draw_depth.z_index_rank(),
            layers:                 BatchRenderLayers(surface.render_layers.clone()),
            shadow:                 surface.shadow_casting.into(),
            pipeline_compatibility: sdf_record_pipeline_compatibility(surface, materials),
            resource_compatibility: materials.resource_compatibility.clone(),
            transparent_sort:       route.transparent_sort,
        }
    }
}

type ChangedSdfPanelInput = Or<(
    Changed<DiegeticPanel>,
    Changed<Transform>,
    Changed<RenderLayers>,
    Changed<Resolved<Lighting>>,
    Changed<Resolved<Sidedness>>,
    Changed<Resolved<ShadowCasting>>,
    Changed<TooltipDepthPriority>,
    Changed<TooltipPaintInsideBorder>,
)>;

#[cfg(test)]
#[derive(Debug, Default, Eq, PartialEq, Resource)]
struct SdfBatchRouteCounts {
    runs:   usize,
    visits: usize,
}

#[cfg(test)]
fn reset_sdf_batch_route_counts(mut counts: ResMut<SdfBatchRouteCounts>) {
    *counts = SdfBatchRouteCounts::default();
}

#[derive(SystemParam)]
struct SdfBatchRouteWakeInputs<'w, 's> {
    panels:        Query<'w, 's, Entity, ChangedSdfPanelInput>,
    tooltip_hosts: Query<'w, 's, (), Or<(Changed<TooltipFor>, Changed<WidgetOf>)>>,
    visibility:    Query<'w, 's, (), Changed<Visibility>>,
}

#[derive(SystemParam)]
struct SdfBatchRouteAssets<'w> {
    standard_materials: Res<'w, Assets<StandardMaterial>>,
    asset_server:       Res<'w, AssetServer>,
    sdf_default:        Res<'w, SdfMaterial>,
    lighting_default:   Res<'w, Lighting>,
    sidedness_default:  Res<'w, Sidedness>,
    visual_overrides:   Res<'w, VisualOverrideIndex>,
}

fn sdf_batch_route_inputs_changed(
    surfaces: Res<ResolvedSdfSurfaceRegistry>,
    wake: SdfBatchRouteWakeInputs,
    assets: SdfBatchRouteAssets,
) -> bool {
    surfaces.has_dirty_panels()
        || !wake.panels.is_empty()
        || !wake.tooltip_hosts.is_empty()
        || !wake.visibility.is_empty()
        || assets.standard_materials.is_changed()
        || assets.sdf_default.is_changed()
        || assets.lighting_default.is_changed()
        || assets.sidedness_default.is_changed()
        || assets.visual_overrides.is_changed()
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
                    .run_if(sdf_batch_route_inputs_changed)
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
                    .before(MaterialTableBufferPrepared)
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
        #[cfg(test)]
        app.init_resource::<SdfBatchRouteCounts>().add_systems(
            PostUpdate,
            reset_sdf_batch_route_counts.before(route_sdf_batch_records),
        );
    }
}

fn route_sdf_batch_records(
    #[cfg(test)] mut run_order: Option<ResMut<SdfDriverRunOrder>>,
    mut build: ResMut<material_table::FrameMaterialTableBuild>,
    mut surfaces: ResMut<ResolvedSdfSurfaceRegistry>,
    assets: SdfBatchRouteAssets,
    panels: Query<(
        Entity,
        &DiegeticPanel,
        Option<&RenderLayers>,
        Option<&Resolved<Lighting>>,
        Option<&Resolved<Sidedness>>,
        Option<&Resolved<ShadowCasting>>,
    )>,
    wake: SdfBatchRouteWakeInputs,
    panel_treatments: SdfPanelTreatmentQueries,
    tooltip_hosts: TooltipHosts,
    mut ancestor_visibility: AncestorVisibility,
    mut store: ResMut<SdfBatchStore>,
    mut perf: ResMut<DiegeticPerfStats>,
    #[cfg(test)] mut counts: Option<ResMut<SdfBatchRouteCounts>>,
) {
    #[cfg(test)]
    material_table::record_sdf_driver_run(&mut run_order, material_table::SDF_DRIVER_ROUTE_RESOLVE);
    #[cfg(test)]
    if let Some(counts) = counts.as_deref_mut() {
        counts.runs += 1;
    }

    let full_pass = !wake.tooltip_hosts.is_empty()
        || !wake.visibility.is_empty()
        || assets.standard_materials.is_changed()
        || assets.sdf_default.is_changed()
        || assets.lighting_default.is_changed()
        || assets.sidedness_default.is_changed()
        || assets.visual_overrides.is_changed();
    let mut dirty_panels = surfaces.take_dirty_panels();
    dirty_panels.extend(wake.panels.iter());
    if full_pass {
        dirty_panels.extend(panels.iter().map(|(entity, ..)| entity));
    }
    let mut stale_panels = HashSet::new();
    let mut dropped = DroppedSdfSurfaces::default();
    let surface_count = surfaces.surfaces().count();
    for panel_entity in dirty_panels {
        #[cfg(test)]
        if let Some(counts) = counts.as_deref_mut() {
            counts.visits += 1;
        }
        build.builder_mut().refresh_sdf_panel(panel_entity);
        let Ok((
            _,
            panel,
            panel_layers,
            resolved_lighting,
            resolved_sidedness,
            resolved_shadow_casting,
        )) = panels.get(panel_entity)
        else {
            // `panel_missing` is counted by the eviction below, from the length
            // of the bucket `remove_panel` drops. Counting it here as well would
            // report every surface twice.
            stale_panels.insert(panel_entity);
            store.retain_panel_records(panel_entity, &HashSet::new());
            continue;
        };
        // Hidden panels keep their frame-table rows and SDF batch records so a
        // `Visibility` change updates the existing GPU buffer allocation.
        let panel_hidden = ancestor_visibility.is_hidden(panel_entity);
        let panel_lighting =
            resolved_lighting.map_or(*assets.lighting_default, |resolved| resolved.0);
        let panel_sidedness =
            resolved_sidedness.map_or(*assets.sidedness_default, |resolved| resolved.0);
        let panel_shadow_casting =
            resolved_shadow_casting.map_or(ShadowCasting::On, |resolved| resolved.0);
        let mut resolved_surfaces = Vec::new();
        for stored_surface in surfaces.panel_surfaces(panel_entity) {
            let mut resolved = stored_surface.as_resolved(
                panel_layers.cloned().unwrap_or(RenderLayers::layer(0)),
                panel_shadow_casting,
            );
            if route_sdf_visual_override(
                stored_surface,
                &mut resolved,
                &assets.visual_overrides,
                &assets.standard_materials,
                &assets.asset_server,
                &assets.sdf_default,
            ) == SdfVisualRoute::EmptyFill
            {
                continue;
            }
            resolved_surfaces.push(PanelSdfSurface {
                surface:        resolved,
                display_target: super::material::PanelDisplayTarget::from(panel.coordinate_space()),
                lighting:       panel_lighting,
                sidedness:      panel_sidedness,
                hidden:         panel_hidden,
            });
        }
        let routed = append_routed_sdf_records(
            build.builder_mut(),
            &resolved_surfaces,
            &assets.standard_materials,
            &assets.asset_server,
            &assets.sdf_default,
            &tooltip_hosts,
            &mut dropped,
        );
        upsert_sdf_batch_records(
            panel_entity,
            routed,
            &panel_treatments.tooltip_depth_priority,
            &panel_treatments.tooltip_edge_treatment,
            &panel_treatments.screen_pixel_edge_snap,
            &tooltip_hosts.panels,
            &mut store,
        );
    }
    record_sdf_surface_stats(
        &mut surfaces,
        &mut perf,
        stale_panels,
        surface_count,
        dropped,
    );
    let _ = surfaces.take_dirty_panels();
}

/// Panel-level treatments applied uniformly to each routed SDF record.
#[derive(SystemParam)]
struct SdfPanelTreatmentQueries<'w, 's> {
    tooltip_depth_priority: Query<'w, 's, (), With<TooltipDepthPriority>>,
    tooltip_edge_treatment: Query<'w, 's, &'static TooltipPaintInsideBorder>,
    screen_pixel_edge_snap: Query<'w, 's, (), With<ScreenPixelEdgeSnap>>,
}

/// A resolved surface with the panel facts that route it into a batch.
struct PanelSdfSurface<'a> {
    surface:        ResolvedSdfSurface<'a>,
    display_target: PanelDisplayTarget,
    lighting:       Lighting,
    sidedness:      Sidedness,
    /// The panel or an ancestor is hidden; its record keeps its rows and
    /// buffer slot.
    hidden:         bool,
}

/// The surfaces `append_routed_sdf_records` gave material rows this frame.
struct RoutedSdfRecords<'s> {
    /// Each routed surface with its material slots and batch route.
    appended: Vec<(
        SdfRecordMaterialSlots,
        &'s ResolvedSdfSurface<'s>,
        SdfRecordRoute,
    )>,
    /// Routed records whose panel is hidden.
    hidden:   HashSet<SdfRecordKey>,
    /// Every routed record; the store drops the rest.
    active:   HashSet<SdfRecordKey>,
}

/// Appends each surface's fill and border rows to the frame material table,
/// counting a surface that gets none in `dropped`.
fn append_routed_sdf_records<'s>(
    builder: &mut FrameMaterialTableBuilder,
    resolved_surfaces: &'s [PanelSdfSurface<'s>],
    standard_materials: &Assets<StandardMaterial>,
    asset_server: &AssetServer,
    sdf_material_default: &SdfMaterial,
    tooltip_hosts: &TooltipHosts<'_, '_>,
    dropped: &mut DroppedSdfSurfaces<FrameWork>,
) -> RoutedSdfRecords<'s> {
    let mut routed = RoutedSdfRecords {
        appended: Vec::new(),
        hidden:   HashSet::new(),
        active:   HashSet::new(),
    };
    for panel_surface in resolved_surfaces {
        let surface = &panel_surface.surface;
        let routing = append_sdf_record_materials(
            builder,
            surface,
            panel_surface.lighting,
            panel_surface.sidedness,
            standard_materials,
            asset_server,
            sdf_material_default,
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
        let record_key = sdf_record_key(surface);
        if panel_surface.hidden {
            routed.hidden.insert(record_key);
        }
        routed.active.insert(record_key);
        let route = SdfRecordRoute {
            display_target:   panel_surface.display_target,
            transparent_sort: tooltip_hosts.transparent_sort(surface),
            batch_sharing:    tooltip_hosts.batch_sharing(surface),
        };
        routed.appended.push((materials, surface, route));
    }
    routed
}

const fn sdf_record_key(surface: &ResolvedSdfSurface<'_>) -> SdfRecordKey {
    SdfRecordKey {
        panel:       surface.panel_entity,
        element:     surface.element_index,
        source_part: surface.source_part,
        role:        surface.record_role,
    }
}

fn upsert_sdf_batch_records(
    panel: Entity,
    routed: RoutedSdfRecords<'_>,
    tooltip_panels: &Query<(), With<render::TooltipDepthPriority>>,
    tooltip_paint_panels: &Query<&TooltipPaintInsideBorder>,
    edge_snap_panels: &Query<(), With<ScreenPixelEdgeSnap>>,
    computed_panels: &Query<&ComputedDiegeticPanel>,
    store: &mut SdfBatchStore,
) {
    let mut records = assign_contiguous_runs(routed.appended);
    let mut frontmost_opaque_by_panel = HashMap::<Entity, f32>::new();
    for record in &records {
        if opaque_fill_depth_push(
            record.batch_key.pipeline_compatibility.alpha,
            record.batch_key.shadow,
        ) > 0.0
        {
            frontmost_opaque_by_panel
                .entry(record.record_key.panel)
                .and_modify(|frontmost| *frontmost = (*frontmost).max(record.clip_depth_nudge))
                .or_insert(record.clip_depth_nudge);
        }
    }
    for record in &mut records {
        let tooltip = tooltip_panels.contains(record.record_key.panel);
        if record.record_key.source_part.is_element()
            && !matches!(record.record_key.role, SdfRecordRole::TooltipShadow(_))
            && let Ok(computed) = computed_panels.get(record.record_key.panel)
            && computed.widget_records().iter().any(|widget| {
                let paint_inside = match widget.authored() {
                    WidgetSpec::EditableField(_) => true,
                    WidgetSpec::Button(button) => button.paints_inside_layout_bounds(),
                    WidgetSpec::Slider(_) => false,
                };
                paint_inside && widget.preorder() == record.record_key.element.get()
            })
        {
            // Keep control paint inside the frame reserved by layout. An exterior
            // AA halo consumes the clear spacing between neighbouring controls.
            record.flags |= SDF_PAINT_INSIDE_BORDER_FLAG;
        }
        if edge_snap_panels.contains(record.record_key.panel) {
            record.flags |= SDF_PAINT_INSIDE_BORDER_FLAG | SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG;
        }
        if let Ok(edge_treatment) = tooltip_paint_panels.get(record.record_key.panel) {
            record.flags |= SDF_PAINT_INSIDE_BORDER_FLAG;
            match edge_treatment {
                TooltipPaintInsideBorder::ScreenPixelGrid => {
                    record.flags |= SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG;
                },
                TooltipPaintInsideBorder::ViewAlignedWorldPixelGrid => {
                    record.flags |= SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG
                        | SDF_TOOLTIP_KEEP_SNAPPED_EDGES_INSIDE_FLAG;
                },
                TooltipPaintInsideBorder::AntialiasedWorldPlane => {},
            }
        }
        if let Some(frontmost) = frontmost_opaque_by_panel.get(&record.record_key.panel)
            && opaque_fill_depth_push(
                record.batch_key.pipeline_compatibility.alpha,
                record.batch_key.shadow,
            ) > 0.0
        {
            let shift = if tooltip {
                *frontmost - OPAQUE_FILL_MAX_CLIP_NUDGE_LAYERS
            } else {
                (*frontmost - OPAQUE_FILL_MAX_CLIP_NUDGE_LAYERS).max(0.0)
            };
            record.clip_depth_nudge -= shift;
        }
        if tooltip {
            record.flags |= SDF_TOOLTIP_DEPTH_PRIORITY_FLAG;
            record.clip_depth_nudge +=
                render::TOOLTIP_NDC_DEPTH_PRIORITY / SDF_CLIP_DEPTH_NUDGE_PER_LAYER;
            record.oit_paint_order += render::TOOLTIP_NDC_DEPTH_PRIORITY;
        }
    }
    place_tooltip_shadows_behind_bodies(&mut records);
    for mut record in records {
        if routed.hidden.contains(&record.record_key) {
            record.mark_hidden();
        }
        store.upsert_record(record);
    }
    store.retain_panel_records(panel, &routed.active);
    for (_, batch) in store.batches_mut() {
        batch.sort_pending_records();
    }
}

/// Puts every tooltip shadow behind the body it belongs to.
///
/// The shadow paints one antialias width under its body's edge, so the body
/// must win that overlap. Without OIT the opaque body's depth test hides the
/// shadow there: the shadow takes the body's clip nudge less
/// `TOOLTIP_SHADOW_BEHIND_BODY_LAYERS`. Under OIT the resolve pass rejects a
/// fragment behind opaque depth, so the shadow sits one `OIT_PAINT_STEP`
/// behind the plane it shares with its body.
fn place_tooltip_shadows_behind_bodies(records: &mut [ResolvedSdfBatchRecord]) {
    let mut body_nudges = HashMap::<(Entity, ElementIndex), f32>::new();
    for record in records.iter().filter(|record| {
        !matches!(record.record_key.role, SdfRecordRole::TooltipShadow(_))
            && record.record_key.source_part.is_element()
    }) {
        body_nudges
            .entry((record.record_key.panel, record.record_key.element))
            .and_modify(|nudge| *nudge = nudge.min(record.clip_depth_nudge))
            .or_insert(record.clip_depth_nudge);
    }
    for shadow in records
        .iter_mut()
        .filter(|record| matches!(record.record_key.role, SdfRecordRole::TooltipShadow(_)))
    {
        if let Some(body_nudge) =
            body_nudges.get(&(shadow.record_key.panel, shadow.record_key.element))
        {
            shadow.clip_depth_nudge = body_nudge - TOOLTIP_SHADOW_BEHIND_BODY_LAYERS;
        }
        shadow.oit_paint_order = -OIT_PAINT_STEP;
    }
}

fn record_sdf_surface_stats(
    surfaces: &mut ResolvedSdfSurfaceRegistry,
    perf: &mut DiegeticPerfStats,
    stale_panels: HashSet<Entity>,
    surface_count: usize,
    mut dropped: DroppedSdfSurfaces<FrameWork>,
) {
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

/// Whether a routed SDF surface has drawable geometry after its widget override.
#[derive(Clone, Copy, Eq, PartialEq)]
enum SdfVisualRoute {
    Retained,
    EmptyFill,
}

fn route_sdf_visual_override<'a>(
    stored_surface: &StoredResolvedSdfSurface,
    surface: &mut ResolvedSdfSurface<'a>,
    visual_overrides: &'a VisualOverrideIndex,
    materials: &Assets<StandardMaterial>,
    asset_server: &AssetServer,
    default_material: &SdfMaterial,
) -> SdfVisualRoute {
    let Some(slot_override) = stored_surface
        .visual_source_element()
        .and_then(|element_index| {
            visual_overrides.get(stored_surface.panel_entity(), element_index.get())
        })
    else {
        return SdfVisualRoute::Retained;
    };
    if slot_override.fill_extent.is_empty() {
        return SdfVisualRoute::EmptyFill;
    }
    apply_sdf_visual_override(
        surface,
        slot_override,
        materials,
        asset_server,
        default_material,
    );
    SdfVisualRoute::Retained
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
    materials: &Assets<StandardMaterial>,
    asset_server: &AssetServer,
    default_material: &SdfMaterial,
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
    if fill_authored && slot_override.dim_fill {
        dim_sdf_material_color(
            &mut surface.fill_material,
            materials,
            asset_server,
            default_material,
        );
    }
    if border_authored && slot_override.dim_border {
        dim_sdf_material_color(
            &mut surface.border_material,
            materials,
            asset_server,
            default_material,
        );
    }
    apply_slider_fill_extent(surface, slot_override);
    if let Some(offset) = slot_override.offset {
        surface.local_transform.translation += offset.extend(0.0);
    }
}

fn apply_slider_fill_extent(
    surface: &mut ResolvedSdfSurface<'_>,
    slot_override: &VisualSlotOverride,
) {
    let Ok(interval) = slot_override.fill_interval() else {
        return;
    };
    let start = interval.start_fraction.clamp(0.0, 1.0);
    let end = interval.end_fraction.clamp(0.0, 1.0);
    let fraction = (end - start).max(0.0);
    let authored_half_size = surface.sdf_half_size;
    let center_fraction = start + end - 1.0;
    let (new_half_size, shift) = match interval.direction {
        SliderDirection::LeftToRight => (
            Vec2::new(authored_half_size.x * fraction, authored_half_size.y),
            Vec2::new(authored_half_size.x * center_fraction, 0.0),
        ),
        SliderDirection::RightToLeft => (
            Vec2::new(authored_half_size.x * fraction, authored_half_size.y),
            Vec2::new(-authored_half_size.x * center_fraction, 0.0),
        ),
        SliderDirection::BottomToTop => (
            Vec2::new(authored_half_size.x, authored_half_size.y * fraction),
            Vec2::new(0.0, authored_half_size.y * center_fraction),
        ),
        SliderDirection::TopToBottom => (
            Vec2::new(authored_half_size.x, authored_half_size.y * fraction),
            Vec2::new(0.0, -authored_half_size.y * center_fraction),
        ),
    };
    surface.sdf_half_size = new_half_size;
    surface.mesh_half_size = new_half_size + Vec2::splat(SDF_AA_PADDING);
    let maximum_radius = new_half_size.x.min(new_half_size.y);
    surface.corner_radii = surface
        .corner_radii
        .map(|radius| radius.min(maximum_radius));
    surface.local_transform.translation += shift.extend(0.0);
}

/// Projects an authored role's visible color into the shared disabled gray.
fn dim_sdf_material_color(
    role: &mut ResolvedSdfMaterial<'_>,
    materials: &Assets<StandardMaterial>,
    asset_server: &AssetServer,
    default_material: &SdfMaterial,
) {
    let color = role.color.or_else(|| {
        let handle = role.base_material.unwrap_or(&default_material.0);
        material::material_asset_for_frame(materials, asset_server, handle, &default_material.0)
            .map(|material| material.base_color)
    });
    role.color = color.map(dim_control_color);
}

fn dim_control_color(color: Color) -> Color {
    let color = color.to_srgba();
    let luminance = color
        .red
        .mul_add(0.2126, color.green.mul_add(0.7152, color.blue * 0.0722));
    let gray = (luminance * 0.65).clamp(0.0, 1.0);
    Color::srgba(gray, gray, gray, color.alpha)
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
    display_textures: Res<super::display_compensation::DisplayCompensationTextures>,
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
        match batch.allocation.resources() {
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
            &display_textures,
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
        let Some(gpu) = batch.allocation.resources() else {
            continue;
        };
        let Some(mut material) = materials.get_mut(&gpu.material) else {
            continue;
        };
        let depth_bias = key.sort_depth_bias();
        if material.base.depth_bias.to_bits() != depth_bias.to_bits() {
            material.base.depth_bias = depth_bias;
        }
        let hardware_depth_bias = key.z_index_rank.hardware_depth_bias();
        if material.extension.hardware_depth_bias != hardware_depth_bias {
            material.extension.hardware_depth_bias = hardware_depth_bias;
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
    // The shader clip is Y-down; world-space bounds use Y-up mesh corners.
    let clip_min = Vec2::new(clip_rect.value.x, -clip_rect.value.w);
    let clip_max = Vec2::new(clip_rect.value.z, -clip_rect.value.y);
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
    display_textures: &super::display_compensation::DisplayCompensationTextures,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<SdfExtendedMaterial>,
    storage_buffers: &mut Assets<ShaderBuffer>,
) {
    let capacity = batch.record_count().max(1).next_power_of_two();
    let records = ShaderBuffer::from(padded_sdf_render_records(
        batch.records(),
        batch.first_draw_order_index(),
        capacity,
    ));
    let mesh_records = ShaderBuffer::from(padded_sdf_mesh_records(capacity));
    let records = storage_buffers.add(records);
    let mesh_records = storage_buffers.add(mesh_records);
    let mesh = meshes.add(inert_sdf_batch_mesh(capacity));
    let material = materials.add(sdf_batch_material(SdfBatchMaterialInput {
        key:           key.clone(),
        records:       records.clone(),
        mesh_records:  mesh_records.clone(),
        display_table: key.display_target.table(display_textures),
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
    batch.allocation = SdfBatchAllocation::Allocated {
        entity:    entity.id(),
        resources: SdfBatchResources {
            records,
            mesh_records,
            mesh,
            material,
            capacity,
        },
    };
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
    let Some(entity) = batch.allocation.entity() else {
        return;
    };
    let Some(current_capacity) = batch.allocation.resources().map(|gpu| gpu.capacity) else {
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
    let records = ShaderBuffer::from(padded_sdf_render_records(
        batch.records(),
        batch.first_draw_order_index(),
        capacity,
    ));
    let mesh_records = ShaderBuffer::from(padded_sdf_mesh_records(capacity));
    let records = storage_buffers.add(records);
    let mesh_records = storage_buffers.add(mesh_records);
    let mesh = meshes.add(inert_sdf_batch_mesh(capacity));
    commands.entity(entity).insert(Mesh3d(mesh.clone()));
    let Some(gpu) = batch.allocation.resources_mut() else {
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
        let Some(gpu) = batch.allocation.resources() else {
            continue;
        };
        if batch.record_count() > gpu.capacity {
            continue;
        }
        let payload = padded_sdf_render_records(
            batch.records(),
            batch.first_draw_order_index(),
            gpu.capacity,
        );
        let bytes = bytemuck::cast_slice(&payload).to_vec();
        let handle = gpu.records.clone();
        batch.record_upload.clear();
        material_table::stage_retained_batch_buffer_upload(&mut staged_uploads, &handle, bytes);
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
    use bevy::camera::RenderTarget;
    use bevy::camera::RenderTargetInfo;
    use bevy::camera::visibility::RenderLayers;
    use bevy::ecs::change_detection::Tick;
    use bevy::ecs::message::Messages;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::image::Image;
    use bevy::picking::hover::PickingInteraction;
    use bevy::prelude::AlphaMode;
    use bevy::prelude::AssetApp;
    use bevy::prelude::Camera;
    use bevy::prelude::ChildOf;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::PerspectiveProjection;
    use bevy::prelude::Projection;
    use bevy::prelude::Quat;
    use bevy::prelude::TransformPlugin;
    use bevy::prelude::UVec2;
    use bevy::prelude::Window;
    use bevy::prelude::default;
    use bevy::render::render_resource::Face;
    use bevy::render::render_resource::FragmentState;
    use bevy::render::render_resource::ShaderSize;
    use bevy::shader::Shader;
    use bevy::shader::ShaderDefVal;
    use bevy::window::PrimaryWindow;
    use bevy::window::WindowRef;
    use hana_kana::ToF32;
    use hana_kana::ToI32;

    use super::material::PanelDisplayTarget;
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
    use crate::render::TooltipDepthPriority;
    use crate::render::display_compensation::DisplayCompensationPlugin;
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
    use crate::widgets::TooltipPaintInsideBorder;
    use crate::widgets::VisualOverrideIndex;
    use crate::widgets::VisualSlotId;
    use crate::widgets::VisualSlotOverride;
    use crate::widgets::WidgetAnchorRect;
    use crate::widgets::WidgetVisualOverrides;
    use crate::widgets::WidgetVisualSlots;
    use crate::widgets::WidgetsPlugin;

    const _: () = assert!(SdfRenderRecord::SHADER_SIZE.get() == 176);

    #[test]
    fn sdf_render_record_direct_storage_matches_encase() -> Result<(), String> {
        let record = SdfRenderRecord {
            transform:        Mat4::from_scale_rotation_translation(
                Vec3::new(2.0, 3.0, 4.0),
                Quat::from_rotation_x(0.4),
                Vec3::new(5.0, 6.0, 7.0),
            ),
            panel_plane:      Vec4::new(0.0, 0.5, 1.0, 2.0),
            half_size:        Vec2::new(8.0, 9.0),
            mesh_half_size:   Vec2::new(10.0, 11.0),
            corner_radii:     Vec4::new(1.0, 2.0, 3.0, 4.0),
            border_widths:    Vec4::new(0.1, 0.2, 0.3, 0.4),
            clip_rect:        Vec4::new(-2.0, -3.0, 12.0, 13.0),
            fill_material:    MaterialSlotId::try_from(7)
                .map_err(|error| format!("{error:?}"))?
                .into(),
            border_material:  MaterialSlotId::try_from(11)
                .map_err(|error| format!("{error:?}"))?
                .into(),
            paint_mask:       SdfPaintMask {
                bits: SdfPaintMask::FILL | SdfPaintMask::BORDER,
            },
            flags:            19,
            clip_depth_nudge: 0.001,
            oit_paint_order:  0.75,
            tooltip_shadow:   Vec2::new(1.5, 2.5),
        };
        assert_eq!(size_of::<SdfRenderRecord>(), 176);
        assert_eq!(
            bytemuck::bytes_of(&record),
            crate::render::encase_storage_bytes(&record)?
        );
        let records = vec![record, SdfRenderRecord::padded()];
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&records),
            crate::render::encase_storage_bytes(&records)?
        );
        Ok(())
    }

    const _: () = assert!(SdfMeshRecord::SHADER_SIZE.get() == 16);

    #[test]
    fn sdf_mesh_record_direct_storage_matches_encase() -> Result<(), String> {
        let record = SdfMeshRecord {
            reserved: UVec4::new(3, 5, 7, 11),
        };
        assert_eq!(size_of::<SdfMeshRecord>(), 16);
        assert_eq!(
            bytemuck::bytes_of(&record),
            crate::render::encase_storage_bytes(&record)?
        );
        Ok(())
    }

    #[test]
    fn sdf_paint_mask_direct_storage_matches_encase() -> Result<(), String> {
        let record = SdfPaintMask {
            bits: SdfPaintMask::FILL | SdfPaintMask::BORDER,
        };
        assert_eq!(
            bytemuck::bytes_of(&record),
            crate::render::encase_storage_bytes(&record)?
        );
        assert_eq!(size_of::<SdfPaintMask>(), 4);
        Ok(())
    }

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
            .add_plugins(DisplayCompensationPlugin)
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

    #[test]
    fn sdf_batch_route_sleeps_and_visits_only_one_changed_panel() {
        let mut app = sdf_pipeline_app();
        let panels: Vec<_> = (0..30)
            .map(|_| spawn_sdf_panel(&mut app, one_surface_tree(), StandardMaterial::default()))
            .collect();
        settle_sdf_pipeline(&mut app);
        app.update();

        let routed_panels: HashSet<_> = sdf_records(&app)
            .iter()
            .map(|record| record.record_key.panel)
            .collect();
        assert_eq!(routed_panels.len(), panels.len());

        assert_eq!(
            app.world().resource::<SdfBatchRouteCounts>(),
            &SdfBatchRouteCounts::default()
        );

        let changed_panel = panels[0];
        assert!(
            app.world_mut()
                .commands()
                .set_tree(
                    changed_panel,
                    single_surface_tree(Color::srgb(0.7, 0.2, 0.1)),
                )
                .is_ok(),
        );
        app.world_mut().flush();
        app.update();

        let counts = app.world().resource::<SdfBatchRouteCounts>();
        assert_eq!(counts.runs, 1);
        assert_eq!(counts.visits, 1);

        app.update();
        app.world_mut()
            .entity_mut(changed_panel)
            .get_mut::<Transform>()
            .expect("panel should carry a transform")
            .translation
            .x += 1.0;
        app.update();

        let counts = app.world().resource::<SdfBatchRouteCounts>();
        assert_eq!(counts.runs, 1, "an authored panel move wakes the route");
        assert_eq!(counts.visits, 1, "only the moved panel is visited");

        app.update();
        app.world_mut()
            .entity_mut(changed_panel)
            .remove::<DiegeticPanel>();
        app.update();

        let counts = app.world().resource::<SdfBatchRouteCounts>();
        assert_eq!(
            counts.runs, 1,
            "panel removal wakes retained-row retirement"
        );
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
        .hide_after(Duration::ZERO);
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

    fn add_tooltip_presentation(app: &mut App) -> Entity {
        let viewport = UVec2::new(800, 600);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: viewport,
            scale_factor:  1.0,
        });
        let mut projection = Projection::Perspective(PerspectiveProjection::default());
        projection.update(800.0, 600.0);
        camera.computed.clip_from_view = projection.get_clip_from_view();
        let transform = Transform::from_xyz(0.0, 0.0, 5.0);
        app.world_mut()
            .spawn((
                camera,
                RenderTarget::Window(WindowRef::Entity(window)),
                RenderLayers::layer(0),
                transform,
                GlobalTransform::from(transform),
            ))
            .id()
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
            .resources()
            .expect("the settled SDF batch should own GPU assets");
        SdfBatchGpuIdentity {
            key:          key.clone(),
            entity:       batch
                .entity()
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
        let camera = add_tooltip_presentation(&mut app);
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
        app.world_mut().entity_mut(widget).insert((
            PickingInteraction::Hovered,
            crate::widgets::TooltipPointerCamera::new(camera),
        ));
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
        let camera = add_tooltip_presentation(&mut app);
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
            .handle()
            .cloned()
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

        app.world_mut().entity_mut(widget).insert((
            PickingInteraction::Hovered,
            crate::widgets::TooltipPointerCamera::new(camera),
        ));
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
            .handle()
            .cloned()
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

    #[test]
    fn screen_sdf_batches_bind_identity_and_world_batches_bind_world_table() {
        let mut app = sdf_pipeline_app();
        app.add_plugins(ScreenSpacePlugin);
        let (identity, world) = {
            let textures = app
                .world()
                .resource::<render::display_compensation::DisplayCompensationTextures>();
            (textures.identity.clone(), textures.world.clone())
        };
        for handle in [&identity, &world] {
            let image = app.world().resource::<Assets<Image>>().get(handle);
            assert!(image.is_some_and(|image| image.texture_descriptor.dimension
                == bevy::render::render_resource::TextureDimension::D3));
        }
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let world_panel = spawn_sdf_panel(
            &mut app,
            single_surface_tree(Color::WHITE),
            StandardMaterial::default(),
        );
        let screen_panel = spawn_sdf_screen_panel(
            &mut app,
            single_surface_tree(Color::WHITE),
            StandardMaterial::default(),
            window,
            1,
            RenderLayers::layer(6),
        );
        settle_sdf_pipeline(&mut app);

        let store = app.world().resource::<SdfBatchStore>();
        let materials = app.world().resource::<Assets<SdfExtendedMaterial>>();
        for (panel, expected_target, expected_table) in [
            (world_panel, PanelDisplayTarget::World, world),
            (screen_panel, PanelDisplayTarget::Screen, identity),
        ] {
            let (key, batch) = store
                .batches()
                .find(|(_, batch)| {
                    batch
                        .records()
                        .iter()
                        .any(|record| record.record_key.panel == panel)
                })
                .expect("panel should have an SDF batch");
            assert_eq!(key.display_target, expected_target);
            let gpu = batch.resources().expect("batch should have a material");
            let bound = materials
                .get(&gpu.material)
                .expect("material should be retained");
            assert_eq!(bound.extension.display_table, expected_table);
        }
    }

    #[test]
    fn filled_panel_interior_skips_border_display_lookup() {
        let shader = include_str!("../shaders/sdf_panel.wesl");
        let coverage = shader.find("var border_alpha = classic_border_alpha;");
        let guard = shader.find("if has_border && border_alpha > 0.0");
        let lookup =
            shader.find("border = vec4<f32>(compensated_unlit_rgb(border.rgb), border.a);");
        assert!(
            matches!((coverage, guard, lookup), (Some(coverage), Some(guard), Some(lookup)) if coverage < guard && guard < lookup)
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
        assert_eq!(table_buffer.handle(), Some(expected_handle));
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

    fn crate_wesl_files() -> Vec<PathBuf> {
        let mut files = Vec::new();
        collect_wesl_files(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        files.sort();
        files
    }

    fn collect_wesl_files(path: &Path, files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).expect("crate source directory should be readable") {
            let entry = entry.expect("crate source entry should be readable");
            let path = entry.path();
            if path.is_dir() {
                collect_wesl_files(&path, files);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "wesl")
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
        let ShaderRef::Handle(handle) = shader_ref else {
            panic!("SdfExtension shader entry point should use the shared SDF shader handle");
        };
        assert_eq!(handle, SDF_PANEL_SHADER_HANDLE);
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
                panel:       Entity::from_bits(1),
                element:     ElementIndex::from(0),
                source_part: SdfSourcePart::Element,
                role:        SdfMaterialRole::Fill,
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
                panel:       Entity::from_bits(1),
                element:     ElementIndex::from(0),
                source_part: SdfSourcePart::Element,
                role:        SdfMaterialRole::Fill,
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
                    display_target: PanelDisplayTarget::World,
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
                    transparent_sort: SdfTransparentSort::PanelRank,
                    batch_sharing: SdfBatchSharing::AnyPanel,
                };
                let material = sdf_batch_material(SdfBatchMaterialInput {
                    key,
                    records: Handle::default(),
                    mesh_records: Handle::default(),
                    display_table: Handle::default(),
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
    fn transparent_fill_material_sorts_before_same_z_panel_text() {
        let mut key = test_batch_key();
        key.shadow = VisualShadow::None;
        key.pipeline_compatibility = PipelineCompatibility::from(&StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            ..Default::default()
        });
        let text_bias = key.z_index_rank.screen_text_depth_bias().get();
        let material = sdf_batch_material(SdfBatchMaterialInput {
            key,
            records: Handle::default(),
            mesh_records: Handle::default(),
            display_table: Handle::default(),
        });

        assert_eq!(material.base.alpha_mode, AlphaMode::Blend);
        assert!(material.base.depth_bias < text_bias);
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
            (
                slots_for_test(0, &material_a),
                &surface_0,
                panel_route(material::PanelDisplayTarget::World),
            ),
            (
                slots_for_test(1, &material_a),
                &surface_1,
                panel_route(material::PanelDisplayTarget::World),
            ),
            (
                slots_for_test(2, &material_b),
                &surface_2,
                panel_route(material::PanelDisplayTarget::World),
            ),
            (
                slots_for_test(3, &material_a),
                &surface_3,
                panel_route(material::PanelDisplayTarget::World),
            ),
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
    fn tooltip_records_never_share_a_run_with_another_panel() {
        let material = StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            ..Default::default()
        };
        let mut materials = Assets::<StandardMaterial>::default();
        let handle = materials.add(material.clone());
        let tooltip_panel = Entity::from_raw_u32(7).expect("valid entity index");
        let host = panel_route(PanelDisplayTarget::World);
        let tooltip = SdfRecordRoute {
            batch_sharing: SdfBatchSharing::TooltipOnly(tooltip_panel),
            ..host
        };
        let routes = [host, tooltip, tooltip, host];
        let surfaces: Vec<_> = routes
            .iter()
            .enumerate()
            .map(|(command_index, route)| {
                let mut surface = resolved_surface_for_test(
                    command_index,
                    &handle,
                    SdfRoleAuthorship::Authored,
                    SdfRoleAuthorship::Unauthored,
                );
                if let SdfBatchSharing::TooltipOnly(panel) = route.batch_sharing {
                    surface.panel_entity = panel;
                }
                surface
            })
            .collect();
        let records = assign_contiguous_runs(
            surfaces
                .iter()
                .zip(routes)
                .enumerate()
                .map(|(index, (surface, route))| (slots_for_test(index, &material), surface, route))
                .collect(),
        );

        let runs: Vec<u32> = records
            .iter()
            .map(|record| record.batch_key.contiguous_drawn_run.value)
            .collect();
        assert_eq!(runs, vec![0, 0, 0, 0], "each panel's records form one run");
        assert_eq!(
            records[0].batch_key, records[3].batch_key,
            "the tooltip's records between them in draw order never split the host's run"
        );
        assert_eq!(records[1].batch_key, records[2].batch_key);
        assert_ne!(records[0].batch_key, records[1].batch_key);
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
            (
                slots_for_test(0, &material),
                &surface_0,
                panel_route(material::PanelDisplayTarget::World),
            ),
            (
                slots_for_test(1, &material),
                &surface_1,
                panel_route(material::PanelDisplayTarget::World),
            ),
            (
                slots_for_test(2, &material),
                &surface_2,
                panel_route(material::PanelDisplayTarget::World),
            ),
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
    fn split_runs_of_one_panel_sort_in_run_order() {
        let blend = StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            ..Default::default()
        };
        let add = StandardMaterial {
            alpha_mode: AlphaMode::Add,
            ..Default::default()
        };
        let mut materials = Assets::<StandardMaterial>::default();
        let blend_handle = materials.add(blend.clone());
        let add_handle = materials.add(add.clone());
        let authored = [
            (&blend_handle, &blend),
            (&add_handle, &add),
            (&blend_handle, &blend),
            (&add_handle, &add),
        ];
        let surfaces: Vec<_> = authored
            .iter()
            .enumerate()
            .map(|(command_index, (handle, _))| {
                resolved_surface_for_test(
                    command_index,
                    handle,
                    SdfRoleAuthorship::Authored,
                    SdfRoleAuthorship::Unauthored,
                )
            })
            .collect();
        let records = assign_contiguous_runs(
            surfaces
                .iter()
                .zip(authored)
                .enumerate()
                .map(|(index, (surface, (_, material)))| {
                    (
                        slots_for_test(index, material),
                        surface,
                        panel_route(PanelDisplayTarget::Screen),
                    )
                })
                .collect(),
        );
        let runs: Vec<u32> = records
            .iter()
            .map(|record| record.batch_key.contiguous_drawn_run.value)
            .collect();
        assert_eq!(runs, vec![0, 1, 2, 3]);

        let rank = DrawZIndexRank::default();
        let mut keys: Vec<SdfBatchKey> =
            records.into_iter().map(|record| record.batch_key).collect();
        let sorts_in_run_order_under = |keys: &[SdfBatchKey], next_tier: f32| {
            let biases: Vec<f32> = keys.iter().map(SdfBatchKey::sort_depth_bias).collect();
            biases.windows(2).all(|pair| pair[0] < pair[1])
                && biases.last().is_some_and(|last| *last < next_tier)
        };
        assert_eq!(
            keys[0].sort_depth_bias().to_bits(),
            rank.screen_depth_bias().get().to_bits(),
            "a panel's first run keeps its rank's bias"
        );
        assert!(
            sorts_in_run_order_under(&keys, rank.screen_panel_shape_depth_bias().get()),
            "each run sorts after the panel's earlier runs and before its panel shapes"
        );

        for key in &mut keys {
            key.transparent_sort = SdfTransparentSort::OverHost(rank);
        }
        assert!(
            sorts_in_run_order_under(&keys, DrawZIndexRank::from(1).screen_depth_bias().get()),
            "tooltip shadow runs sort in run order and before the host's next rank"
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
                panel:       Entity::from_bits(1),
                element:     ElementIndex::from(0),
                source_part: SdfSourcePart::Element,
                role:        SdfRecordRole::Whole,
            },
            draw_order:       CommandIndex::from(0),
            fill_source:      SdfMaterialSourceKey {
                panel:       Entity::from_bits(1),
                element:     ElementIndex::from(0),
                source_part: SdfSourcePart::Element,
                role:        SdfMaterialRole::Fill,
            },
            border_source:    SdfMaterialSourceKey {
                panel:       Entity::from_bits(1),
                element:     ElementIndex::from(0),
                source_part: SdfSourcePart::Element,
                role:        SdfMaterialRole::Border,
            },
            draw_depth:       draw_depth_for_test(0),
            batch_key:        test_batch_key(),
            local_transform:  Transform::IDENTITY,
            transform:        Mat4::from_translation(Vec3::new(2.0, 3.0, 4.0)),
            panel_plane:      WorldPanelPlane::from_panel(
                &GlobalTransform::from(Transform::from_xyz(2.0, 3.0, 4.0)),
                0.0,
            ),
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
            oit_paint_order:  0.0,
            flags:            0,
        };
        batch.upsert_record(record);

        let (min, max) = batch.world_bounds().expect("clipped record has bounds");

        assert_eq!(min, Vec3::new(1.0, 2.0, 4.0));
        assert_eq!(max, Vec3::new(4.0, 5.0, 4.0));
    }

    #[test]
    fn top_only_sdf_clip_keeps_top_world_bounds() {
        let bounds = clipped_local_bounds(
            MeshHalfSize {
                value: Vec2::splat(5.0),
            },
            LocalClipRect {
                value: Vec4::new(-5.0, -5.0, 5.0, -2.0),
            },
        );
        assert_eq!(bounds, Some((Vec2::new(-5.0, 2.0), Vec2::new(5.0, 5.0))));
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
            records_before[0].oit_paint_order.to_bits(),
            crate::render::constants::OIT_PAINT_STEP.to_bits()
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
            records_after[0].oit_paint_order.to_bits(),
            crate::render::constants::OIT_PAINT_STEP.to_bits()
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
    fn tooltip_opaque_oit_depth_clears_host_at_grazing_angles() {
        let shader = include_str!("../shaders/sdf_panel.wesl");
        assert!(shader.contains("(record.flags & TOOLTIP_DEPTH_PRIORITY_FLAG) != 0u"));
        assert!(shader.contains("clip.z += TOOLTIP_NDC_DEPTH_PRIORITY * clip.w"));
        let host_distance = 10.0_f32;
        let near = 0.1_f32;
        for angle in [75.0_f32, 85.0] {
            let clearance_on_ray = 0.005 * angle.to_radians().cos();
            let tooltip_depth =
                near / (host_distance - clearance_on_ray) + render::TOOLTIP_NDC_DEPTH_PRIORITY;
            for (host_part, stack_depth) in
                [("knob", OIT_PAINT_STEP), ("focus", 2.0 * OIT_PAINT_STEP)]
            {
                let host_depth = near / host_distance + stack_depth;
                assert!(
                    tooltip_depth > host_depth + OIT_PAINT_STEP,
                    "tooltip opaque prepass depth must clear the {host_part} at {angle} degrees"
                );
            }
        }
    }

    #[test]
    fn world_tooltip_surface_clears_host_without_hiding_its_text() {
        let mut app = sdf_pipeline_app();
        let host = spawn_sdf_panel(
            &mut app,
            stacked_surface_tree(Color::WHITE, Color::BLACK),
            StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                ..default()
            },
        );
        let mut tooltip_tree = LayoutBuilder::with_root(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(Color::WHITE),
        );
        tooltip_tree.text(("Tooltip", TextStyle::new(12.0)));
        let tooltip = spawn_sdf_panel(
            &mut app,
            tooltip_tree.build(),
            StandardMaterial {
                alpha_mode: AlphaMode::Opaque,
                ..default()
            },
        );
        app.world_mut()
            .entity_mut(tooltip)
            .insert(TooltipDepthPriority);
        settle_sdf_pipeline(&mut app);

        let records = sdf_records(&app);
        let host_front = records
            .iter()
            .filter(|record| record.record_key.panel == host)
            .map(|record| record.oit_paint_order)
            .fold(0.0_f32, f32::max);
        let tooltip_surface = records
            .iter()
            .find(|record| record.record_key.panel == tooltip)
            .expect("tooltip should have a surface record");
        assert_eq!(
            tooltip_surface.flags & SDF_TOOLTIP_DEPTH_PRIORITY_FLAG,
            SDF_TOOLTIP_DEPTH_PRIORITY_FLAG,
        );
        let surface_nudge = tooltip_surface.clip_depth_nudge * SDF_CLIP_DEPTH_NUDGE_PER_LAYER;
        assert_eq!(
            tooltip_surface.clip_depth_nudge.to_bits(),
            (OPAQUE_FILL_MAX_CLIP_NUDGE_LAYERS
                + render::TOOLTIP_NDC_DEPTH_PRIORITY / SDF_CLIP_DEPTH_NUDGE_PER_LAYER)
                .to_bits(),
        );
        assert!(
            render::TOOLTIP_NDC_DEPTH_PRIORITY > host_front,
            "tooltip OIT path must clear the host's nearest content layer"
        );
        assert!(
            tooltip_surface.oit_paint_order > host_front,
            "tooltip's OIT path receives the same front priority"
        );
        let computed = app.world().get::<ComputedDiegeticPanel>(tooltip);
        let text_nudge = computed
            .and_then(|computed| computed.layout().solved().map(|solved| (computed, solved)))
            .and_then(|(computed, solved)| {
                solved
                    .commands
                    .iter()
                    .enumerate()
                    .find_map(|(index, command)| {
                        matches!(command.kind, RenderCommandKind::Text { .. })
                            .then(|| computed.draw_order().depth_for(index))
                            .flatten()
                    })
            })
            .map(|depth| {
                depth.clip_depth_nudge().get().mul_add(
                    render::TEXT_CLIP_DEPTH_NUDGE_PER_LAYER,
                    render::TOOLTIP_NDC_DEPTH_PRIORITY,
                )
            });
        assert!(text_nudge.is_some());
        if let Some(text_nudge) = text_nudge {
            assert!(
                text_nudge > surface_nudge,
                "tooltip text must clear its opaque surface"
            );
        }

        for radius in [0.5_f32, 5.0, 50.0, 500.0] {
            let near = radius * 0.001;
            for distance in [radius, radius * 3.0] {
                for face_cosine in [1.0_f32, 0.5] {
                    let plane_gap = 0.005 * face_cosine;
                    let host_depth = near / distance + host_front;
                    let surface_depth = near / (distance - plane_gap) + surface_nudge;
                    assert!(
                        surface_depth > host_depth,
                        "radius={radius}, distance={distance}, cosine={face_cosine}: surface={surface_depth:e}, host={host_depth:e}"
                    );
                }
            }
        }
    }

    #[test]
    fn tooltip_shadow_runs_under_its_body_edge_and_draws_behind_it() {
        let mut app = sdf_pipeline_app();
        let tree = LayoutBuilder::with_root(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(Color::WHITE),
        )
        .build();
        let tooltip = spawn_sdf_panel(
            &mut app,
            tree,
            StandardMaterial {
                alpha_mode: AlphaMode::Opaque,
                ..default()
            },
        );
        app.world_mut().entity_mut(tooltip).insert((
            TooltipPaintInsideBorder::ViewAlignedWorldPixelGrid,
            TooltipDepthPriority,
        ));
        settle_sdf_pipeline(&mut app);

        let surfaces = app.world().resource::<ResolvedSdfSurfaceRegistry>();
        let roles: Vec<SdfRecordRole> = surfaces
            .surfaces()
            .filter(|surface| surface.panel_entity() == tooltip)
            .map(|surface| surface.as_resolved(RenderLayers::layer(0), ShadowCasting::Off))
            .filter(|surface| surface.element_index == ElementIndex::default())
            .map(|surface| surface.record_role)
            .collect();
        assert!(
            matches!(roles.first(), Some(SdfRecordRole::TooltipShadow(_))),
            "the shadow sorts ahead of its body: {roles:?}"
        );

        let records = sdf_records(&app);
        let tooltip_records = || {
            records
                .iter()
                .filter(|record| record.record_key.panel == tooltip)
        };
        let shadow = tooltip_records()
            .find(|record| matches!(record.record_key.role, SdfRecordRole::TooltipShadow(_)))
            .expect("tooltip shadow record");
        let body = tooltip_records()
            .find(|record| !matches!(record.record_key.role, SdfRecordRole::TooltipShadow(_)))
            .expect("tooltip body record");
        assert!(
            shadow.clip_depth_nudge < body.clip_depth_nudge,
            "without OIT the opaque body hides the shadow under its edge"
        );
        assert!(
            (shadow.clip_depth_nudge + TOOLTIP_SHADOW_BEHIND_BODY_LAYERS)
                * SDF_CLIP_DEPTH_NUDGE_PER_LAYER
                >= render::TOOLTIP_NDC_DEPTH_PRIORITY,
            "the shadow stays within its margin of the tooltip depth band's front"
        );
        assert!(
            shadow.oit_paint_order < 0.0,
            "under OIT the shadow sits behind its body's plane"
        );

        let shader = include_str!("../shaders/sdf_panel.wesl");
        assert!(
            shader.contains("if body_dist < -aa_width(body_dist) {\n        discard;"),
            "the shadow paints from one antialias width inside the body's edge"
        );
    }

    #[test]
    fn edge_snap_flags_route_for_tooltips_and_opt_in_screen_panels() {
        let mut app = sdf_pipeline_app();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let tree = LayoutBuilder::with_root(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(Color::WHITE)
                .border(Border::all(Px(1.0), Color::WHITE)),
        )
        .build();
        let world = spawn_sdf_panel(&mut app, tree.clone(), StandardMaterial::default());
        app.world_mut().entity_mut(world).insert((
            TooltipPaintInsideBorder::ViewAlignedWorldPixelGrid,
            TooltipDepthPriority,
        ));
        let screen = spawn_sdf_screen_panel(
            &mut app,
            tree,
            StandardMaterial::default(),
            window,
            0,
            RenderLayers::layer(0),
        );
        app.world_mut()
            .entity_mut(screen)
            .insert(TooltipPaintInsideBorder::ScreenPixelGrid);
        let marked_screen = spawn_sdf_screen_panel(
            &mut app,
            multiple_edge_snap_surface_tree(),
            StandardMaterial::default(),
            window,
            0,
            RenderLayers::layer(0),
        );
        app.world_mut()
            .entity_mut(marked_screen)
            .insert(ScreenPixelEdgeSnap);
        let ordinary_control = spawn_sdf_screen_panel(
            &mut app,
            ordinary_edge_snap_control_tree(),
            StandardMaterial::default(),
            window,
            0,
            RenderLayers::layer(0),
        );
        settle_sdf_pipeline(&mut app);

        let records = sdf_records(&app);
        assert_tooltip_edge_snap_records(&records, world, screen);
        assert_marked_edge_snap_records(&records, marked_screen);
        assert_ordinary_edge_snap_records(&records, ordinary_control);
        assert_edge_snap_shader_contract(include_str!("../shaders/sdf_panel.wesl"));
    }

    #[test]
    fn pop_up_button_paints_inside_layout_without_changing_ordinary_coverage() {
        let mut app = sdf_pipeline_app();
        let mut builder = LayoutBuilder::new(Px(320.0), Px(180.0));
        builder.text(crate::Button::new().pop_up(
            "choice",
            "Opaque",
            "Transparent",
            &crate::ControlMetrics::from_label_size(Px(13.0)),
        ));
        let popup = spawn_sdf_panel(&mut app, builder.build(), StandardMaterial::default());
        let ordinary = spawn_sdf_panel(
            &mut app,
            ordinary_edge_snap_control_tree(),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);

        let records = sdf_records(&app);
        let popup_records: Vec<_> = records
            .iter()
            .filter(|record| record.record_key.panel == popup)
            .collect();
        assert!(
            !popup_records.is_empty(),
            "the pop-up must publish SDF paint"
        );
        for record in popup_records {
            assert_eq!(
                record.flags
                    & (SDF_PAINT_INSIDE_BORDER_FLAG
                        | SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG
                        | SDF_TOOLTIP_DEPTH_PRIORITY_FLAG
                        | SDF_TOOLTIP_KEEP_SNAPPED_EDGES_INSIDE_FLAG),
                SDF_PAINT_INSIDE_BORDER_FLAG,
                "pop-up paint stays inside its bounds without snapping or tooltip treatment"
            );
        }
        assert_ordinary_edge_snap_records(&records, ordinary);
    }

    fn multiple_edge_snap_surface_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(Px(320.0), Px(180.0));
        builder.with(
            El::column()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(Color::WHITE)
                .border(Border::all(Px(1.0), Color::WHITE)),
            |builder| {
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(Color::WHITE),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    fn ordinary_edge_snap_control_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(Px(320.0), Px(180.0));
        builder.with(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(Color::WHITE)
                .border(Border::all(Px(1.0), Color::WHITE))
                .button("ordinary-control"),
            |_| {},
        );
        builder.build()
    }

    fn assert_marked_edge_snap_records(records: &[ResolvedSdfBatchRecord], panel: Entity) {
        let marked_records: Vec<_> = records
            .iter()
            .filter(|record| record.record_key.panel == panel)
            .collect();
        assert!(
            marked_records.len() > 1,
            "the opted-in screen-panel fixture must exercise every record"
        );
        for record in marked_records {
            assert_eq!(
                record.flags & (SDF_PAINT_INSIDE_BORDER_FLAG | SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG),
                SDF_PAINT_INSIDE_BORDER_FLAG | SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG,
                "every opted-in screen-panel record must paint inside its snapped edge"
            );
            assert_eq!(
                record.flags & SDF_TOOLTIP_DEPTH_PRIORITY_FLAG,
                0,
                "the public edge-snap marker must not make a screen panel a tooltip"
            );
        }
    }

    fn assert_ordinary_edge_snap_records(records: &[ResolvedSdfBatchRecord], panel: Entity) {
        let ordinary_control_records: Vec<_> = records
            .iter()
            .filter(|record| record.record_key.panel == panel)
            .collect();
        assert!(
            !ordinary_control_records.is_empty(),
            "the ordinary-control fixture must publish an SDF record"
        );
        for record in ordinary_control_records {
            assert_eq!(
                record.flags & (SDF_PAINT_INSIDE_BORDER_FLAG | SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG),
                0,
                "an ordinary control must keep the unflagged coverage path"
            );
        }
    }

    fn assert_tooltip_edge_snap_records(
        records: &[ResolvedSdfBatchRecord],
        world: Entity,
        screen: Entity,
    ) {
        for panel in [world, screen] {
            let record = records
                .iter()
                .find(|record| {
                    record.record_key.panel == panel
                        && !matches!(record.record_key.role, SdfRecordRole::TooltipShadow(_))
                })
                .expect("tooltip SDF record");
            assert_eq!(
                record.flags & SDF_PAINT_INSIDE_BORDER_FLAG,
                SDF_PAINT_INSIDE_BORDER_FLAG,
                "tooltip paint flag must reach the {panel:?} SDF record",
            );
            assert!(
                record.flags & SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG != 0,
                "both view-aligned tooltips must snap their outer edges",
            );
            assert_eq!(
                record.flags & SDF_TOOLTIP_KEEP_SNAPPED_EDGES_INSIDE_FLAG != 0,
                panel == world,
                "only the world tooltip must keep snapped edges inside",
            );
            assert!(
                record.mesh_half_size.value.x > record.half_size.value.x,
                "SDF mesh must include horizontal antialias room"
            );
            assert!(
                record.mesh_half_size.value.y > record.half_size.value.y,
                "SDF mesh must include vertical antialias room"
            );
            assert!(
                record.border_widths.value.x > 0.0,
                "SDF record must carry the border width"
            );
        }
        let screen_record = records
            .iter()
            .find(|record| {
                record.record_key.panel == screen
                    && !matches!(record.record_key.role, SdfRecordRole::TooltipShadow(_))
            })
            .expect("screen tooltip SDF record");
        assert_eq!(
            screen_record.flags & SDF_TOOLTIP_DEPTH_PRIORITY_FLAG,
            0,
            "screen tooltip must not receive world depth priority"
        );
    }

    fn assert_edge_snap_shader_contract(shader: &str) {
        assert!(shader.contains("const SDF_PAINT_INSIDE_BORDER_FLAG: u32 = 2u;"));
        assert!(shader.contains("const SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG: u32 = 4u;"));
        assert!(shader.contains("const TOOLTIP_KEEP_SNAPPED_EDGES_INSIDE_FLAG: u32 = 8u;"));
        assert!(shader.contains(
            "@if(SDF_TOOLTIP_SHADOW) {\n    var pbr_input = tooltip_shadow_pbr_input(in, is_front, record, local);\n}\n@if(!SDF_TOOLTIP_SHADOW) {\n    let view_aligned_edge_snap = (record.flags & SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG) != 0u;"
        ));
        assert!(
            shader.contains("if paint_inside_border && outer_dist >= 0.0 {\n        discard;"),
            "shader must discard flagged pixels beyond the border outer edge"
        );
        assert!(
            shader
                .contains("select(aa_width(dist), 0.5 * distance_field_band(dist), paint_inside)"),
            "shader must centre a one-pixel antialias ramp on each edge of flagged tooltips"
        );
        assert!(
            shader.contains("vec4<f32>(pixel_size.y, pixel_size.x, pixel_size.y, pixel_size.x)"),
            "tooltip frame widths must follow the physical pixel derivatives"
        );
        assert!(
            shader.contains("let edge_snap_border_clip = view_aligned_edge_snap")
                && shader.contains(
                    "vec4<f32>(\n            -record.border_widths.w,\n            -record.border_widths.z,\n            record.border_widths.y,\n            record.border_widths.x,"
                ),
            "only view-aligned edge-snap records may restore clipped authored border coverage"
        );
        assert!(
            shader.contains("let near_grid = select(round(near_screen), inward_near_grid,")
                && shader.contains("let far_grid = select(round(far_screen), inward_far_grid,"),
            "world edges must snap inward while screen edges snap to the nearest pixel"
        );
    }

    #[test]
    fn tooltip_edge_snap_keeps_ordinary_panel_derivatives_in_uniform_control_flow() {
        let shader = include_str!("../shaders/sdf_panel.wesl");
        assert!(
            !shader.contains("if view_aligned_edge_snap"),
            "record-specific snapping must not branch before the shared edge derivatives"
        );
        assert!(
            shader.contains("let edge_local = select(")
                && shader.contains("let effective_half_size = select("),
            "snapped and ordinary geometry must be selected without control-flow divergence"
        );
    }

    #[test]
    fn opaque_sdf_fill_after_draw_position_six_stays_under_text_occlusion_slack() {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(El::overlay().size(100.0, 50.0), |builder| {
            for _ in 0..8 {
                builder.with(El::new().size(100.0, 50.0).background(Color::WHITE), |_| {});
            }
        });
        let mut app = sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            builder.build(),
            StandardMaterial {
                alpha_mode: AlphaMode::Opaque,
                ..default()
            },
        );
        settle_sdf_pipeline(&mut app);
        let records = sdf_records(&app);
        assert_eq!(records.len(), 8);
        let frontmost = records
            .iter()
            .map(|record| record.clip_depth_nudge)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            frontmost * SDF_CLIP_DEPTH_NUDGE_PER_LAYER < crate::render::constants::OIT_PAINT_STEP,
            "late opaque fills must not hide text resting on them"
        );
        assert_eq!(
            (records[7].clip_depth_nudge - records[0].clip_depth_nudge).to_bits(),
            7.0_f32.to_bits(),
            "the common depth shift preserves order among opaque fills"
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
        assert!(image_record.draw_depth.oit_paint_order().get() < border_record.oit_paint_order);
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
        assert!(fill_record.oit_paint_order <= image_record.draw_depth.oit_paint_order().get());

        assert_eq!(
            border_record.batch_key.pipeline_compatibility.alpha,
            BatchAlphaMode::Blend
        );
        assert_eq!(
            border_record.clip_depth_nudge.to_bits(),
            border_record.draw_depth.clip_depth_nudge().get().to_bits()
        );
        assert!(image_record.draw_depth.oit_paint_order().get() < border_record.oit_paint_order);
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
        assert!(fill_record.oit_paint_order <= image_record.draw_depth.oit_paint_order().get());

        assert_eq!(
            border_record.batch_key.pipeline_compatibility.alpha,
            BatchAlphaMode::Blend
        );
        assert_eq!(
            border_record.clip_depth_nudge.to_bits(),
            border_record.draw_depth.clip_depth_nudge().get().to_bits()
        );
        assert!(image_record.draw_depth.oit_paint_order().get() < border_record.oit_paint_order);
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

        assert_eq!(sdf_records(&app), []);
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
    fn a_hidden_ancestor_masks_sdf_records_the_frame_it_hides() {
        let mut app = sdf_pipeline_app();
        let panel = spawn_sdf_panel(
            &mut app,
            single_surface_tree(Color::WHITE),
            StandardMaterial::default(),
        );
        let tool = app
            .world_mut()
            .spawn((Transform::default(), Visibility::Inherited))
            .id();
        app.world_mut().entity_mut(panel).insert(ChildOf(tool));
        settle_sdf_pipeline(&mut app);
        let shown_mask = sdf_records(&app)[0].paint_mask.bits();
        assert_ne!(shown_mask, 0);

        app.world_mut().entity_mut(tool).insert(Visibility::Hidden);
        app.update();
        let hidden_records = sdf_records(&app);
        assert_eq!(hidden_records.len(), 1);
        assert_eq!(
            hidden_records[0].paint_mask.bits(),
            0,
            "hiding the panel's parent masks its fill in the same frame"
        );

        app.world_mut()
            .entity_mut(tool)
            .insert(Visibility::Inherited);
        app.update();
        assert_eq!(sdf_records(&app)[0].paint_mask.bits(), shown_mask);
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
        assert!(records[0].oit_paint_order < records[1].oit_paint_order);
    }

    #[test]
    fn switch_knob_and_slider_thumb_resolve_in_front_of_their_tracks() {
        const MINIMUM_QUANTA_MARGIN: u32 = 2;
        let track = Color::srgb(0.28, 0.73, 0.43);
        let knob = Color::srgb(0.95, 0.95, 0.96);
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(El::column(), |builder| {
            builder.with(
                El::overlay()
                    .size(Mm(65.0), Mm(28.0))
                    .background(track)
                    .switch("switch"),
                |builder| {
                    builder.with(
                        El::new()
                            .size(Mm(23.0), Mm(23.0))
                            .background(knob)
                            .switch_knob(),
                        |_| {},
                    );
                },
            );
            builder.with(
                El::overlay()
                    .size(Mm(100.0), Mm(10.0))
                    .background(track)
                    .slider("slider", 0.0..=1.0),
                |builder| {
                    builder.with(
                        El::new()
                            .size(Mm(4.0), Mm(10.0))
                            .background(knob)
                            .slider_thumb(),
                        |_| {},
                    );
                    builder.with(
                        El::new()
                            .size(Mm(2.0), Mm(10.0))
                            .background(knob)
                            .slider_marker(),
                        |_| {},
                    );
                },
            );
        });
        let mut app = sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            builder.build(),
            StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                ..Default::default()
            },
        );
        settle_sdf_pipeline(&mut app);

        let records = sdf_records(&app);
        assert_eq!(records.len(), 5);
        let (switch_track, switch_knob) = (&records[0], &records[1]);
        let (slider_track, slider_thumb, slider_marker) = (&records[2], &records[3], &records[4]);
        // The near plane tracks zoom, so the focus distance is zoom itself.
        let near = 7.754_f32 * 0.001;
        let depth = near / 7.754;
        for (under, over, pair) in [
            (switch_track, switch_knob, "switch knob over its track"),
            (slider_track, slider_thumb, "slider thumb over its track"),
            (slider_thumb, slider_marker, "slider marker over its thumb"),
        ] {
            let physical = depth.to_bits();
            let under_packed = (under.oit_paint_order
                * crate::render::constants::OIT_PACKED_PAINT_RANGE)
                .round()
                .to_u32();
            let over_packed = (over.oit_paint_order
                * crate::render::constants::OIT_PACKED_PAINT_RANGE)
                .round()
                .to_u32();
            assert!((physical, over_packed) > (physical, under_packed));
            let margin = over_packed - under_packed;
            assert!(
                margin >= MINIMUM_QUANTA_MARGIN,
                "{pair}: {margin} OIT paint quanta",
            );
        }
        let shader = include_str!("../shaders/sdf_panel.wesl");
        assert!(shader.contains("oit_draw_layer(oit_pos, out.color, record.oit_paint_order)"));
    }

    #[test]
    fn switch_oit_records_share_the_panel_plane_after_the_knob_moves() {
        let metrics = crate::ControlMetrics::from_label_size(13.0);
        let switch = metrics.switch;
        let mut builder = LayoutBuilder::new(100.0, 100.0);
        builder.with(
            El::overlay()
                .size(switch.track_width, switch.track_height)
                .padding(Padding::all(switch.track_padding))
                .alignment(AlignX::Left, AlignY::Center)
                .background(Color::BLACK)
                .border(Border::all(switch.border_width, Color::BLACK))
                .corner_radius(switch.track_radius())
                .switch("power"),
            |builder| {
                builder.with(
                    El::new()
                        .size(switch.thumb.width, switch.thumb.height)
                        .background(Color::WHITE)
                        .corner_radius(switch.thumb.corner_radius())
                        .switch_knob(),
                    |_| {},
                );
            },
        );
        let mut app = sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            builder.build(),
            StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                ..default()
            },
        );
        settle_sdf_pipeline(&mut app);
        let mut records = sdf_records(&app);
        assert_eq!(records.len(), 2);
        let panel_pose = GlobalTransform::from(Transform {
            translation: Vec3::new(17.23, -2.51, 8.0),
            rotation:    Quat::from_rotation_y(0.41),
            scale:       Vec3::new(1.7, 0.9, 1.1),
        });
        records[0].update_world_transform(&panel_pose);
        records[1].local_transform.translation.x += 0.003;
        records[1].update_world_transform(&panel_pose);
        let (track, knob) = (&records[0], &records[1]);
        assert_eq!(
            track.panel_plane.coefficients.to_array().map(f32::to_bits),
            knob.panel_plane.coefficients.to_array().map(f32::to_bits),
        );
        assert_ne!(track.transform, knob.transform);
        // The shader reconstructs the same base depth from this common plane.
        // Only the existing stack-layer offset differs at an overlapping pixel.
        for depth in [0.001_f32, 0.25, 0.5, 0.999_9] {
            let packed = |offset: f32| {
                (offset * crate::render::constants::OIT_PACKED_PAINT_RANGE)
                    .round()
                    .to_u32()
            };
            let under = packed(track.oit_paint_order);
            let over = packed(knob.oit_paint_order);
            assert!(over >= under + 2, "switch OIT layer ties at {depth}");
            assert!((depth.to_bits(), over) > (depth.to_bits(), under));
        }
        assert_switch_plane_survives_an_unchanged_upsert(track.clone());
    }

    fn assert_switch_plane_survives_an_unchanged_upsert(record: ResolvedSdfBatchRecord) {
        let expected = record.panel_plane.coefficients.to_array().map(f32::to_bits);
        let mut batch = SdfBatch::default();
        batch.upsert_record(record.clone());
        batch.record_upload.clear();
        let mut rebuilt = record;
        rebuilt.transform = rebuilt.local_transform.to_matrix();
        rebuilt.panel_plane = WorldPanelPlane::from_panel(&GlobalTransform::IDENTITY, 0.0);
        batch.upsert_record(rebuilt);
        assert_eq!(
            batch.records()[0]
                .panel_plane
                .coefficients
                .to_array()
                .map(f32::to_bits),
            expected,
        );
        assert!(!batch.record_upload.is_set());
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
            .join("sdf_material_table.wesl");
        let helper = fs::read_to_string(&helper_path).expect("SDF material helper should load");
        let body_range = function_body_range(&helper, "fn pbr_input_from_material_table");
        let body = &helper[body_range.clone()];
        let mut outside = String::new();
        outside.push_str(&helper[..body_range.start]);
        outside.push_str(&helper[body_range.end..]);

        assert!(helper.contains("fn pbr_input_from_material_table"));
        assert_eq!(outside.matches("material_table[").count(), 0);
        // Two mutually exclusive in-helper reads: the depth/shadow prepass uses
        // opacity and UVs, while the main pass applies the full material row.
        assert_eq!(body.matches("material_table[").count(), 2);
        for path in crate_wesl_files() {
            let source = fs::read_to_string(&path).expect("crate WESL file should load");
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
        let helper = include_str!("../shaders/sdf_material_table.wesl");
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
        let shader = include_str!("../shaders/sdf_panel.wesl");

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
        let helper = include_str!("../shaders/sdf_material_table.wesl");
        let shader = include_str!("../shaders/sdf_panel.wesl");
        let constants = include_str!("material_table.wesl");

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
                "import hana_diegetic::sdf_material_table::pbr_input_from_material_table;"
            )
        );
        assert_ordered(
            shader,
            &[
                "fn fill_alpha_for_prepass",
                "pbr_input_from_material_table(",
                "@if(PREPASS_PIPELINE)",
                "@fragment",
            ],
        );
        assert_ordered(
            shader,
            &[
                "@if(!PREPASS_PIPELINE) {\n@fragment",
                "let fill_pbr = pbr_input_from_material_table(",
                "let border_pbr = pbr_input_from_material_table(",
            ],
        );
        // Specialized per-pipeline layout assertions stay in the GPU/visual pass because
        // `RenderPipelineDescriptor` layouts require a render-device-backed specialization path.
    }

    #[test]
    fn stripped_material_group_fallback_is_present_for_prepass_review() {
        let shader = include_str!("../shaders/sdf_panel.wesl");
        let helper = include_str!("../shaders/sdf_material_table.wesl");
        let mut descriptor = RenderPipelineDescriptor {
            fragment: Some(FragmentState::default()),
            ..Default::default()
        };

        add_sdf_stripped_material_group_def(&mut descriptor, SdfPipelineMode::MeshAttributes);
        assert_eq!(descriptor.vertex.shader_defs, []);
        assert_eq!(
            descriptor
                .fragment
                .as_ref()
                .expect("fragment state should exist")
                .shader_defs,
            []
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
        assert!(shader.contains("@if(SDF_STRIPPED_MATERIAL_GROUP)"));
        assert!(shader.contains("@if(PREPASS_PIPELINE)"));
        assert!(helper.contains("@if(SDF_STRIPPED_MATERIAL_GROUP)"));
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

    #[test]
    fn only_a_tooltip_shadow_batch_compiles_the_shadow_fragment_path() {
        let shader = include_str!("../shaders/sdf_panel.wesl");
        let shadow_def = ShaderDefVal::from("SDF_TOOLTIP_SHADOW");
        let fragment_defs = |key: SdfBatchKey| {
            let material = sdf_batch_material(SdfBatchMaterialInput {
                key,
                records: Handle::default(),
                mesh_records: Handle::default(),
                display_table: Handle::default(),
            });
            let extension_key = SdfExtensionKey::from(&material.extension);
            let mut descriptor = RenderPipelineDescriptor {
                fragment: Some(FragmentState::default()),
                ..Default::default()
            };
            add_sdf_fragment_paint_def(&mut descriptor, extension_key.fragment_paint);
            assert_eq!(descriptor.vertex.shader_defs, []);
            descriptor
                .fragment
                .expect("fragment state should exist")
                .shader_defs
        };

        let surface = fragment_defs(test_batch_key());
        let shadow = fragment_defs(SdfBatchKey {
            transparent_sort: SdfTransparentSort::OverHost(DrawZIndexRank::default()),
            ..test_batch_key()
        });

        assert!(
            !surface.contains(&shadow_def),
            "surface fragment defs {surface:?} compile the shadow path"
        );
        assert!(
            shadow.contains(&shadow_def),
            "shadow fragment defs {shadow:?} lack the shadow path"
        );
        assert!(shader.contains("@if(SDF_TOOLTIP_SHADOW)"));
        // The driver inlines lighting once per call site, so a second call
        // doubles every SDF pipeline's cold compile.
        assert_eq!(shader.matches("apply_pbr_lighting(").count(), 1);
    }

    #[test]
    fn only_an_sdf_batch_with_lit_records_compiles_pbr_lighting() {
        let shader = include_str!("../shaders/sdf_panel.wesl");
        let lighting_def = ShaderDefVal::from("SDF_PBR_LIGHTING");
        let fragment_defs = |material: &StandardMaterial| {
            let material = sdf_batch_material(SdfBatchMaterialInput {
                key:           sdf_batch_key_for_material(material, VisualShadow::None),
                records:       Handle::default(),
                mesh_records:  Handle::default(),
                display_table: Handle::default(),
            });
            let extension_key = SdfExtensionKey::from(&material.extension);
            let mut descriptor = RenderPipelineDescriptor {
                fragment: Some(FragmentState::default()),
                ..Default::default()
            };
            add_sdf_fragment_lighting_def(&mut descriptor, extension_key.fragment_lighting);
            descriptor
                .fragment
                .expect("fragment state should exist")
                .shader_defs
        };

        let unlit = fragment_defs(&StandardMaterial {
            unlit: true,
            ..Default::default()
        });
        let lit = fragment_defs(&StandardMaterial::default());

        assert!(
            !unlit.contains(&lighting_def),
            "unlit-only fragment defs {unlit:?} compile PBR lighting"
        );
        assert!(
            lit.contains(&lighting_def),
            "lit fragment defs {lit:?} lack PBR lighting"
        );
        assert!(shader.contains("@if(!PREPASS_PIPELINE && SDF_PBR_LIGHTING)"));
        assert!(shader.contains("import bevy_pbr::render::pbr_functions::apply_pbr_lighting;"));
    }

    const fn panel_route(display_target: PanelDisplayTarget) -> SdfRecordRoute {
        SdfRecordRoute {
            display_target,
            transparent_sort: SdfTransparentSort::PanelRank,
            batch_sharing: SdfBatchSharing::AnyPanel,
        }
    }

    fn test_batch_key() -> SdfBatchKey {
        SdfBatchKey {
            display_target:         PanelDisplayTarget::World,
            z_index:                0.into(),
            z_index_rank:           DrawZIndexRank::default(),
            batch_family:           DrawBatchFamily::SdfSurface,
            layers:                 BatchRenderLayers(RenderLayers::layer(0)),
            shadow:                 VisualShadow::Cast,
            contiguous_drawn_run:   ContiguousDrawnRun::default(),
            pipeline_compatibility: PipelineCompatibility::from(&StandardMaterial::default()),
            resource_compatibility: ResourceCompatibility::from(&StandardMaterial::default()),
            transparent_sort:       SdfTransparentSort::PanelRank,
            batch_sharing:          SdfBatchSharing::AnyPanel,
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
            source_part:     SdfSourcePart::Element,
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
                            batch.entity().expect("widget part batch should be spawned"),
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
    fn disabled_surface_fallback_dims_authored_fill_and_restores_it() {
        let mut app = widget_sdf_pipeline_app();
        spawn_sdf_panel(
            &mut app,
            slotted_widget_fill_tree(0),
            StandardMaterial::default(),
        );
        settle_sdf_pipeline(&mut app);
        let widget = styled_widget(&mut app);
        let records = sdf_records(&app);
        assert_fill_row_colors(&app, &records, &[WIDGET_FILL_COLOR, PEER_FILL_COLOR]);

        set_slot_override(
            &mut app,
            widget,
            VisualSlotOverride {
                dim_fill: true,
                ..VisualSlotOverride::default()
            },
        );
        app.update();
        let records = sdf_records(&app);
        assert_fill_row_colors(
            &app,
            &records,
            &[super::dim_control_color(WIDGET_FILL_COLOR), PEER_FILL_COLOR],
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
            source_part: SdfSourcePart::Element,
            role: SdfMaterialRole::Fill,
        };
        let expected_record_key = SdfRecordKey {
            panel,
            element: ElementIndex::from(3),
            source_part: SdfSourcePart::Element,
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
            source_part: SdfSourcePart::Element,
            role: SdfMaterialRole::Fill,
        };
        let expected_record_key = SdfRecordKey {
            panel,
            element: ElementIndex::from(3),
            source_part: SdfSourcePart::Element,
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
            source_part: SdfSourcePart::Element,
            role: SdfMaterialRole::Fill,
        };
        let expected = HashSet::from([
            SdfMaterialSourceKey {
                panel,
                element: ElementIndex::from(1),
                source_part: SdfSourcePart::Divider(0),
                role: SdfMaterialRole::Fill,
            },
            SdfMaterialSourceKey {
                panel,
                element: ElementIndex::from(1),
                source_part: SdfSourcePart::Divider(1),
                role: SdfMaterialRole::Fill,
            },
        ]);
        let records = sdf_records(&app);
        let divider_keys: HashSet<_> = records
            .iter()
            .map(|record| record.fill_source)
            .filter(|source| !source.source_part.is_element())
            .collect();
        let divider_slots: HashSet<_> = records
            .iter()
            .filter(|record| !record.fill_source.source_part.is_element())
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
            .filter(|(source, _)| !source.source_part.is_element())
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
            .filter(|source| !source.source_part.is_element())
            .collect();
        assert_eq!(divider_keys, expected);
        let updated_divider_command_indices: HashMap<_, _> = sdf_records(&app)
            .iter()
            .map(|record| (record.fill_source, record.draw_order))
            .filter(|(source, _)| !source.source_part.is_element())
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
            record.fill_source.source_part.is_element()
                && fill_row_color(&app, record) == linear_color(SLOT_OVERRIDE_COLOR)
        }) else {
            panic!();
        };
        let dividers: Vec<_> = records
            .iter()
            .filter(|record| !record.fill_source.source_part.is_element())
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
                .and_then(|(_, batch)| batch.entity())
                .expect("the widget fill batch should be spawned");
            let (_, peer_batch) = store
                .batches()
                .find(|(key, _)| key.z_index == 1.into())
                .expect("the peer fill batch should exist");
            let peer = peer_batch.entity().expect("peer batch should be spawned");
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
            .and_then(|(_, batch)| batch.entity())
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
            .map(|(_, batch)| (batch.entity(), batch.records().to_vec()))
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

    fn slider_with_fill_tree(direction: SliderDirection) -> LayoutTree {
        let (width, height, fill_width, fill_height) = match direction {
            SliderDirection::LeftToRight | SliderDirection::RightToLeft => (40.0, 16.0, 40.0, 4.0),
            SliderDirection::BottomToTop | SliderDirection::TopToBottom => (16.0, 40.0, 4.0, 40.0),
        };
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::overlay()
                .size(width, height)
                .alignment(AlignX::Center, AlignY::Center)
                .widget(
                    "level",
                    Slider::new(0.0..=1.0).value(0.0).direction(direction),
                ),
            |builder| {
                builder.with(
                    El::new()
                        .size(fill_width, fill_height)
                        .background(SLOT_OVERRIDE_COLOR)
                        .slider_fill(),
                    |_| {},
                );
                builder.with(
                    El::new()
                        .size(8.0, 8.0)
                        .background(PEER_FILL_COLOR)
                        .slider_thumb(),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    #[test]
    fn slider_fill_keeps_its_leading_edge_and_meets_the_thumb_at_each_value() {
        for direction in [SliderDirection::LeftToRight, SliderDirection::BottomToTop] {
            let mut app = widget_sdf_pipeline_app();
            spawn_sdf_panel(
                &mut app,
                slider_with_fill_tree(direction),
                StandardMaterial::default(),
            );
            settle_sdf_pipeline(&mut app);
            let widget = styled_widget(&mut app);
            let slots = app
                .world()
                .get::<WidgetVisualSlots>(widget)
                .expect("slider visual slots");
            let fill_index = slots
                .element_index(VisualSlotId::SLIDER_FILL)
                .expect("fill slot");
            let thumb_index = slots
                .element_index(VisualSlotId::SLIDER_THUMB)
                .expect("thumb slot");
            let mut authored_leading_edge: Option<f32> = None;
            for value in [0.0, 0.5, 1.0] {
                if value > 0.0 {
                    let mut state = app
                        .world_mut()
                        .get_mut::<SliderState>(widget)
                        .expect("slider state");
                    assert!(state.set_value(value).expect("value applies"));
                    app.update();
                }
                let records = sdf_records(&app);
                let fill = records
                    .iter()
                    .find(|record| record.record_key.element.get() == fill_index)
                    .expect("fill record");
                let thumb = records
                    .iter()
                    .find(|record| record.record_key.element.get() == thumb_index)
                    .expect("thumb record");
                let (leading_edge, trailing_edge, thumb_center) = match direction {
                    SliderDirection::LeftToRight | SliderDirection::RightToLeft => (
                        fill.local_transform.translation.x - fill.half_size.value.x,
                        fill.local_transform.translation.x + fill.half_size.value.x,
                        thumb.local_transform.translation.x,
                    ),
                    SliderDirection::BottomToTop | SliderDirection::TopToBottom => (
                        fill.local_transform.translation.y - fill.half_size.value.y,
                        fill.local_transform.translation.y + fill.half_size.value.y,
                        thumb.local_transform.translation.y,
                    ),
                };
                if let Some(authored) = authored_leading_edge {
                    assert!((leading_edge - authored).abs() < 1e-6);
                } else {
                    authored_leading_edge = Some(leading_edge);
                }
                assert!(
                    (trailing_edge - thumb_center).abs() < 1e-6,
                    "{direction:?} at {value}: fill must meet the thumb center"
                );
            }
            set_slot_overrides(
                &mut app,
                widget,
                [(
                    VisualSlotId::SLIDER_FILL,
                    VisualSlotOverride::slider_fill(direction, 0.0),
                )],
            );
            app.update();
            assert!(
                sdf_records(&app)
                    .iter()
                    .all(|record| record.record_key.element.get() != fill_index),
                "zero fill extent must emit no SDF record"
            );
        }
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
            .filter_map(|(_, batch)| batch.resources().map(|gpu| gpu.records.clone()))
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
            .and_then(|(_, batch)| batch.entity())
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
            .and_then(|(_, batch)| batch.entity())
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

    /// Reverse-Z depth steps from `behind` up to `front`.
    fn depth_ulps_between(behind: f32, front: f32) -> u32 { front.to_bits() - behind.to_bits() }

    #[test]
    fn tooltip_shadow_sits_whole_ulps_behind_its_body_inside_the_tooltip_band() {
        let margin = TOOLTIP_SHADOW_BEHIND_BODY_LAYERS * SDF_CLIP_DEPTH_NUDGE_PER_LAYER;
        // Screen-ortho panels sit near 0.53; the top of the range has the widest ulp.
        for body_depth in [0.53_f32, 0.999_9, 1.0, 0.25, 0.001] {
            let shadow_depth = body_depth - margin;
            assert!(
                depth_ulps_between(shadow_depth, body_depth)
                    >= TOOLTIP_SHADOW_BEHIND_BODY_DEPTH_ULPS.to_u32(),
                "shadow at {shadow_depth} is not {TOOLTIP_SHADOW_BEHIND_BODY_DEPTH_ULPS} ulps \
                 behind its body at {body_depth}",
            );
        }
        assert!(
            margin < 0.01 * render::TOOLTIP_NDC_DEPTH_PRIORITY,
            "the shadow margin {margin} leaves the tooltip band",
        );
    }

    #[test]
    fn tooltip_shadow_sorts_between_its_host_text_and_next_rank_at_its_body_hardware_bias() {
        let rank = DrawZIndexRank::default();
        for host in [0_usize, 3] {
            let shadow = SdfBatchKey {
                transparent_sort: SdfTransparentSort::OverHost(DrawZIndexRank::from(host)),
                ..test_batch_key()
            };
            let host_text = DrawZIndexRank::from(host).screen_text_depth_bias().get();
            let next_fill = DrawZIndexRank::from(host + 1).screen_depth_bias().get();
            assert!(
                (host_text..next_fill).contains(&shadow.sort_depth_bias())
                    && shadow.sort_depth_bias() > host_text,
                "shadow over a rank-{host} host sorts at {}, not between {host_text} and {next_fill}",
                shadow.sort_depth_bias()
            );
        }
        let shadow = SdfBatchKey {
            transparent_sort: SdfTransparentSort::OverHost(rank),
            ..test_batch_key()
        };
        let material = sdf_batch_material(SdfBatchMaterialInput {
            key:           shadow,
            records:       Handle::default(),
            mesh_records:  Handle::default(),
            display_table: Handle::default(),
        });
        assert_eq!(
            material.extension.hardware_depth_bias,
            rank.hardware_depth_bias()
        );
        assert_eq!(
            material.extension.hardware_depth_bias,
            test_batch_key().sort_depth_bias().to_i32(),
        );
    }
}
