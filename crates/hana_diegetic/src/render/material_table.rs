//! Retained material table for diegetic batched render records.
//!
//! Each material source keeps one `MaterialSlotId` while it exists. Producers
//! update that row while building records, and removed sources wait two frames
//! before their rows can be reused. A replacement GPU buffer is prepared for
//! one render frame before batch materials bind it.

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::hash::Hash;
use std::time::Duration;
use std::time::Instant;

use bevy::asset::AssetEventSystems;
use bevy::color::Color;
use bevy::log::warn_once;
use bevy::pbr::MeshMaterial3d;
use bevy::pbr::StandardMaterial;
use bevy::pbr::StandardMaterialUniform;
use bevy::prelude::Added;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::LinearRgba;
use bevy::prelude::Mat3;
use bevy::prelude::Or;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectResource;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::SystemSet;
use bevy::prelude::TransformSystems;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Vec4;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::prelude::info;
use bevy::prelude::warn;
use bevy::render::Extract;
use bevy::render::ExtractSchedule;
use bevy::render::Render;
use bevy::render::RenderApp;
use bevy::render::RenderSystems;
use bevy::render::erased_render_asset::prepare_erased_assets;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_asset::prepare_assets;
use bevy::render::render_resource::AsBindGroupShaderType;
use bevy::render::render_resource::ShaderType;
use bevy::render::renderer::RenderDevice;
use bevy::render::renderer::RenderQueue;
use bevy::render::storage::GpuShaderBuffer;
use bevy::render::storage::ShaderBuffer;
use bevy::render::texture::GpuImage;
use hana_kana::ToF32;
use hana_kana::ToU32;
use hana_kana::ToUsize;

use super::PathExtendedMaterial;
use super::SdfExtendedMaterial;
use super::batch_key::PipelineCompatibility;
use super::batch_key::ResourceCompatibility;
#[cfg(test)]
use super::fill_batch::SdfMaterialRole;
use super::fill_batch::SdfMaterialSourceKey;
use super::image_material::ImageExtendedMaterial;
use super::panel_shapes::PanelShapeRenderKey;
use crate::panel::DiegeticPerfStats;
use crate::panel::LifetimeTotal;
use crate::text::RunStorageKey;

/// Path material uniform binding used by `PathExtension::uniforms`.
const PATH_UNIFORM_BINDING: u32 = 100;
/// Path shared curve table storage binding used by `PathExtension::curves`.
const PATH_CURVES_BINDING: u32 = 101;
/// Path shared band table storage binding used by `PathExtension::bands`.
const PATH_BANDS_BINDING: u32 = 102;
/// Path shared path-record table storage binding used by `PathExtension::path_records`.
const PATH_RECORDS_BINDING: u32 = 103;
/// Path per-instance storage binding used by `PathExtension::instances`.
pub(super) const PATH_INSTANCES_BINDING: u32 = 104;
/// Path per-run storage binding used by `PathExtension::run_records`.
pub(super) const PATH_RUN_RECORDS_BINDING: u32 = 105;
/// Shared material-slot table storage binding used by path and SDF extensions.
pub(super) const MATERIAL_TABLE_BINDING: u32 = 106;
/// Batched SDF render-record storage binding used by the batched SDF fill route.
pub(super) const SDF_RENDER_RECORDS_BINDING: u32 = 107;
/// Batched SDF mesh-record storage binding used by the batched SDF fill route.
pub(super) const SDF_MESH_BINDING: u32 = 108;

/// GPU sentinel for an absent SDF fill or border material table read.
pub(super) const INVALID_GPU_MATERIAL_SLOT: u32 = u32::MAX;

#[cfg(test)]
pub(super) const SDF_DRIVER_ROUTE_RESOLVE: &str = "route/resolve";
#[cfg(test)]
pub(super) const SDF_DRIVER_WORLD_TRANSFORMS: &str = "world-transforms";
#[cfg(test)]
pub(super) const SDF_DRIVER_RECONCILE_SPAWN: &str = "reconcile/spawn";
#[cfg(test)]
const SDF_DRIVER_REGISTER: &str = "register";
#[cfg(test)]
pub(super) const SDF_DRIVER_BOUNDS: &str = "bounds";
#[cfg(test)]
pub(super) const SDF_DRIVER_COMMIT: &str = "commit";

#[cfg(test)]
pub(super) const SDF_DRIVER_RUN_ORDER: [&str; 6] = [
    SDF_DRIVER_ROUTE_RESOLVE,
    SDF_DRIVER_WORLD_TRANSFORMS,
    SDF_DRIVER_RECONCILE_SPAWN,
    SDF_DRIVER_REGISTER,
    SDF_DRIVER_COMMIT,
    SDF_DRIVER_BOUNDS,
];

#[cfg(test)]
#[derive(Default, Resource)]
pub(super) struct SdfDriverRunOrder {
    pub(super) names:                   Vec<&'static str>,
    pub(super) registered_batch_entity: bool,
}

#[cfg(test)]
pub(super) fn record_sdf_driver_run(
    run_order: &mut Option<ResMut<SdfDriverRunOrder>>,
    name: &'static str,
) {
    if let Some(run_order) = run_order.as_deref_mut() {
        run_order.names.push(name);
    }
}

const DEFAULT_TABLE_CAPACITY: u32 = 64;
/// Reserves room for a full live-source re-key while prior source rows retire.
const CAPACITY_HEADROOM_DIVISOR: u32 = 1;
const MATERIAL_SLOT_RETIREMENT_FRAMES: u64 = 2;
const MATERIAL_TABLE_STRESS_FRAMES: usize = 16;
const MATERIAL_TABLE_WARMUP_FRAMES: usize = 4;
const MEDIUM_MEASUREMENT_ENTRIES: usize = 5_000;
const SMALL_MEASUREMENT_ENTRIES: usize = 128;
const STRESS_MEASUREMENT_ENTRIES: usize = 10_000;
const TOPOLOGY_CHURN_PERCENT: usize = 10;

/// Stable material-table row id returned to CPU record builders.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MaterialSlotId(
    /// Row inside `FrameMaterialTable::rows` while its source remains live.
    u32,
);

impl MaterialSlotId {
    /// Returns the row index stored by GPU records.
    #[must_use]
    pub(super) const fn as_u32(self) -> u32 { self.0 }

    /// Builds a row id only when `raw` names a row in the current frame table.
    const fn from_raw_in_table(raw: u32, row_count: u32) -> Result<Self, MaterialSlotIdError> {
        if raw == INVALID_GPU_MATERIAL_SLOT {
            return Err(MaterialSlotIdError::ReservedSentinel);
        }
        if raw >= row_count {
            return Err(MaterialSlotIdError::OutOfRange { raw, row_count });
        }
        Ok(Self(raw))
    }
}

impl From<MaterialSlotId> for u32 {
    fn from(slot: MaterialSlotId) -> Self { slot.0 }
}

impl TryFrom<u32> for MaterialSlotId {
    type Error = MaterialSlotIdError;

    fn try_from(raw: u32) -> Result<Self, Self::Error> {
        if raw == INVALID_GPU_MATERIAL_SLOT {
            Err(MaterialSlotIdError::ReservedSentinel)
        } else {
            Ok(Self(raw))
        }
    }
}

/// Failure reason for converting a raw GPU row value back to a CPU row id.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MaterialSlotIdError {
    /// The raw value is `INVALID_GPU_MATERIAL_SLOT`, reserved for `NotAuthored`.
    ReservedSentinel,
    /// The raw value is not present in the current frame table.
    OutOfRange {
        /// Raw row value supplied by a GPU-facing record.
        raw:       u32,
        /// Number of live rows in the extracted frame table.
        row_count: u32,
    },
}

/// Opaque GPU record row id for SDF fill and border material references.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, ShaderType)]
pub(crate) struct GpuMaterialSlotId {
    /// Raw row index or `INVALID_GPU_MATERIAL_SLOT` sentinel stored in GPU records.
    raw: u32,
}

impl GpuMaterialSlotId {
    /// Returns the raw row value written to GPU records.
    #[must_use]
    pub(super) const fn as_u32(self) -> u32 { self.raw }
}

impl From<MaterialSlotId> for GpuMaterialSlotId {
    fn from(slot: MaterialSlotId) -> Self { Self { raw: slot.as_u32() } }
}

/// CPU-side SDF material presence before a GPU record is built.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum SdfPaintMaterial {
    /// The fill or border role was authored and reads this material-table row.
    Authored(MaterialSlotId),
    /// The fill or border role was not authored and must not read the table.
    NotAuthored,
}

impl SdfPaintMaterial {
    /// Converts the CPU role state to the single GPU row encoding.
    #[must_use]
    pub(super) const fn to_gpu(self) -> GpuMaterialSlotId {
        match self {
            Self::Authored(slot) => GpuMaterialSlotId { raw: slot.as_u32() },
            Self::NotAuthored => GpuMaterialSlotId {
                raw: INVALID_GPU_MATERIAL_SLOT,
            },
        }
    }

    /// Converts a GPU row encoding back to the CPU role state for tests.
    fn from_gpu(gpu_slot: GpuMaterialSlotId, row_count: u32) -> Result<Self, MaterialSlotIdError> {
        let raw = gpu_slot.as_u32();
        if raw == INVALID_GPU_MATERIAL_SLOT {
            Ok(Self::NotAuthored)
        } else {
            MaterialSlotId::from_raw_in_table(raw, row_count).map(Self::Authored)
        }
    }
}

/// Scalar/vector material values stored in the shared frame table.
///
/// Classification source: Bevy's `StandardMaterialUniform` value fields are
/// table data when they can vary per record without changing a bind group,
/// pipeline specialization, pass route, or required mesh attributes.
/// `flags`, texture handles/presence bits, UV-channel selectors,
/// `alpha_cutoff`, `double_sided`, `cull_mode`, `unlit`, `fog_enabled`,
/// normal-map/depth-map requirements, `opaque_render_method`, and deferred pass
/// id stay in compatibility keys. `StandardMaterial::depth_bias` is ignored
/// here because diegetic draw-order types own depth and OIT offsets. Parallax
/// scalar fields without a depth-map resource, lightmap exposure, and
/// feature-gated texture-channel values whose textures are absent remain
/// deferred until a render-family design intentionally consumes them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect, ShaderType)]
pub(crate) struct MaterialSlotValues {
    /// `StandardMaterialUniform::base_color` read by material-table shaders.
    pub base_color:                     Vec4,
    /// `StandardMaterialUniform::emissive`; `.w` stores emissive exposure weight.
    pub emissive:                       Vec4,
    /// `StandardMaterialUniform::attenuation_color` read by PBR table sampling.
    pub attenuation_color:              Vec4,
    /// `StandardMaterialUniform::uv_transform` composed with element-local box UVs.
    pub uv_transform:                   Mat3,
    /// `StandardMaterialUniform::reflectance`, the Bevy-computed specular tint.
    pub reflectance:                    Vec3,
    /// `StandardMaterialUniform::roughness` table value.
    pub roughness:                      f32,
    /// `StandardMaterialUniform::metallic` table value.
    pub metallic:                       f32,
    /// `StandardMaterialUniform::diffuse_transmission` table value.
    pub diffuse_transmission:           f32,
    /// `StandardMaterialUniform::specular_transmission` table value.
    pub specular_transmission:          f32,
    /// `StandardMaterialUniform::thickness` table value.
    pub thickness:                      f32,
    /// `StandardMaterialUniform::ior` table value.
    pub ior:                            f32,
    /// `StandardMaterialUniform::attenuation_distance` table value.
    pub attenuation_distance:           f32,
    /// `StandardMaterialUniform::clearcoat` table value.
    pub clearcoat:                      f32,
    /// `StandardMaterialUniform::clearcoat_perceptual_roughness` table value.
    pub clearcoat_perceptual_roughness: f32,
    /// `StandardMaterialUniform::anisotropy_strength` table value.
    pub anisotropy_strength:            f32,
    /// `StandardMaterialUniform::anisotropy_rotation` as Bevy's computed unit vector.
    pub anisotropy_rotation:            Vec2,
}

impl MaterialSlotValues {
    /// Builds the frame-table row from a fully classified Bevy material uniform.
    #[must_use]
    const fn from_standard_material_uniform(uniform: StandardMaterialUniform) -> Self {
        let StandardMaterialUniform {
            base_color,
            emissive,
            attenuation_color,
            uv_transform,
            reflectance,
            roughness,
            metallic,
            diffuse_transmission,
            specular_transmission,
            thickness,
            ior,
            attenuation_distance,
            clearcoat,
            clearcoat_perceptual_roughness,
            anisotropy_strength,
            anisotropy_rotation,
            flags: _flags_compatibility_data,
            alpha_cutoff: _alpha_cutoff_compatibility_data,
            parallax_depth_scale: _parallax_depth_scale_table_when_appendix_d,
            max_parallax_layer_count: _max_parallax_layer_count_table_when_appendix_d,
            lightmap_exposure: _deferred_until_lightmap_design,
            max_relief_mapping_search_steps: _max_relief_mapping_search_steps_table_when_appendix_d,
            deferred_lighting_pass_id: _deferred_pass_compatibility_data,
        } = uniform;

        Self {
            base_color,
            emissive,
            attenuation_color,
            uv_transform,
            reflectance,
            roughness,
            metallic,
            diffuse_transmission,
            specular_transmission,
            thickness,
            ior,
            attenuation_distance,
            clearcoat,
            clearcoat_perceptual_roughness,
            anisotropy_strength,
            anisotropy_rotation,
        }
    }

    /// Returns the shader-layout row size used for storage-buffer accounting.
    #[must_use]
    fn shader_size_bytes() -> usize {
        usize::try_from(Self::min_size().get()).unwrap_or(usize::MAX)
    }
}

impl From<&StandardMaterial> for MaterialSlotValues {
    fn from(material: &StandardMaterial) -> Self {
        let images = RenderAssets::<GpuImage>::default();
        Self::from_standard_material_uniform(material.as_bind_group_shader_type(&images))
    }
}

/// Projection result for a resolved `StandardMaterial` source.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MaterialSlotCandidate {
    /// Scalar/vector row values appended to `FrameMaterialTableBuilder`.
    pub values:                 MaterialSlotValues,
    /// Pipeline facts used by SDF/path batch keys instead of the table row.
    pub pipeline_compatibility: PipelineCompatibility,
    /// Texture and UV-channel facts copied into batch render materials.
    pub resource_compatibility: ResourceCompatibility,
}

impl From<&StandardMaterial> for MaterialSlotCandidate {
    fn from(material: &StandardMaterial) -> Self {
        Self {
            values:                 MaterialSlotValues::from(material),
            pipeline_compatibility: PipelineCompatibility::from(material),
            resource_compatibility: ResourceCompatibility::from(material),
        }
    }
}

/// Identity used to retain one table row across producer traversals and frames.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum MaterialSourceKey {
    /// One standalone analytic-line probe entity.
    AnalyticLine(Entity),
    /// One authored SDF fill or border role.
    Sdf(SdfMaterialSourceKey),
    /// One text run owned by a label entity.
    Text(RunStorageKey),
    /// One retained panel-line render record.
    PanelLine(PanelShapeRenderKey),
    /// One synthetic measurement entry.
    Measurement(u32),
    /// One unit-test SDF producer entry.
    #[cfg(test)]
    TestSdf(u32),
    /// One unit-test text producer entry.
    #[cfg(test)]
    TestText(u32),
    /// One unit-test panel-line producer entry.
    #[cfg(test)]
    TestPanelLine(u32),
    /// One raw-row unit-test entry.
    #[cfg(test)]
    Test(u32),
}

impl From<SdfMaterialSourceKey> for MaterialSourceKey {
    fn from(key: SdfMaterialSourceKey) -> Self { Self::Sdf(key) }
}

impl From<Entity> for MaterialSourceKey {
    fn from(entity: Entity) -> Self { Self::AnalyticLine(entity) }
}

impl From<RunStorageKey> for MaterialSourceKey {
    fn from(key: RunStorageKey) -> Self { Self::Text(key) }
}

impl From<PanelShapeRenderKey> for MaterialSourceKey {
    fn from(key: PanelShapeRenderKey) -> Self { Self::PanelLine(key) }
}

/// Temporary append-time material input shared by table producers.
pub(super) trait MaterialSlotInput {
    /// Source key returned with the appended slot for record-builder routing.
    type Key: Copy + Eq + Hash + Into<MaterialSourceKey>;

    /// Returns the producer-specific source key for this material role.
    fn key(&self) -> Self::Key;

    /// Projects the resolved material role into table row values and compatibility keys.
    fn material_slot_candidate(&self) -> MaterialSlotCandidate;
}

/// Successful material-slot append output for a producer key.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct MaterialSlotAppended<K> {
    /// Source key supplied by the append input.
    pub key:                    K,
    /// Stable row assigned by `FrameMaterialTableBuilder`.
    pub slot:                   MaterialSlotId,
    /// Pipeline compatibility copied beside the row id for batch selection.
    pub pipeline_compatibility: PipelineCompatibility,
    /// Resource compatibility copied beside the row id for batch-material creation.
    pub resource_compatibility: ResourceCompatibility,
}

/// Result of appending one material-table role.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum MaterialSlotAppend<K> {
    /// The row was updated and the returned slot remains valid while the source exists.
    Appended(MaterialSlotAppended<K>),
    /// The current frame table reached the storage-buffer row limit.
    DroppedLimit,
}

/// Result of appending one raw row into `FrameMaterialTableBuilder`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FrameMaterialSlotAppend {
    /// The row was updated and this retained slot was assigned.
    Appended(MaterialSlotId),
    /// The current frame table reached the storage-buffer row limit.
    DroppedLimit,
}

/// Appends one projected material role through the shared table contract.
pub(super) fn append_material_slot<T>(
    builder: &mut FrameMaterialTableBuilder,
    input: &T,
) -> MaterialSlotAppend<T::Key>
where
    T: MaterialSlotInput,
{
    let key = input.key();
    let candidate = input.material_slot_candidate();
    match builder.upsert_values(key.into(), candidate.values) {
        FrameMaterialSlotAppend::Appended(slot) => {
            MaterialSlotAppend::Appended(MaterialSlotAppended {
                key,
                slot,
                pipeline_compatibility: candidate.pipeline_compatibility,
                resource_compatibility: candidate.resource_compatibility,
            })
        },
        FrameMaterialSlotAppend::DroppedLimit => MaterialSlotAppend::DroppedLimit,
    }
}

/// Retained material-table payload extracted with record buffers.
#[derive(Clone, Debug, Default, Reflect, Resource)]
#[reflect(Resource)]
pub(super) struct FrameMaterialTable {
    /// Rows indexed by the stable ids stored in render records.
    rows:      Vec<MaterialSlotValues>,
    /// Sources that wrote or retained a row in this frame.
    live_rows: usize,
}

impl FrameMaterialTable {
    /// Returns the material rows for extraction and upload.
    #[must_use]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Phase 3 record extraction will read rows through this accessor"
        )
    )]
    pub(super) fn rows(&self) -> &[MaterialSlotValues] { &self.rows }

    /// Returns the highest retained row extent in this frame.
    #[must_use]
    pub(super) const fn row_count(&self) -> usize { self.rows.len() }

    /// Returns the number of material sources that own rows in this frame.
    #[must_use]
    pub(super) const fn live_row_count(&self) -> usize { self.live_rows }

    /// Returns the allocated row capacity of the CPU table vector.
    #[must_use]
    const fn capacity(&self) -> usize { self.rows.capacity() }

    fn padded_rows(&self, capacity: u32) -> Vec<MaterialSlotValues> {
        let mut rows = Vec::with_capacity(capacity.to_usize());
        rows.extend_from_slice(&self.rows);
        rows.resize(
            capacity.to_usize().max(self.rows.len()),
            MaterialSlotValues::default(),
        );
        rows
    }

    fn encoded_rows(&self, capacity: u32) -> Vec<u8> {
        ShaderBuffer::from(self.padded_rows(capacity))
            .data
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct MaterialSlotEntry {
    source:     Option<MaterialSourceKey>,
    values:     MaterialSlotValues,
    seen_frame: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RetiredMaterialSlot {
    slot:              MaterialSlotId,
    reusable_at_frame: u64,
}

/// Retained material-table builder shared by all producers.
#[derive(Debug)]
pub(super) struct FrameMaterialTableBuilder {
    /// Stable entries indexed by `MaterialSlotId`.
    entries:           Vec<MaterialSlotEntry>,
    /// Live source-to-row assignments.
    source_slots:      HashMap<MaterialSourceKey, MaterialSlotId>,
    /// Rows waiting for the prior extraction snapshots to expire.
    retired:           VecDeque<RetiredMaterialSlot>,
    /// New source identities rejected by the active GPU capacity.
    rejected:          HashSet<MaterialSourceKey>,
    /// New assignments made during the current producer-update window.
    new_assignments:   Vec<(MaterialSourceKey, MaterialSlotId)>,
    /// Maximum row count allowed by the current storage-buffer capacity limit.
    row_limit:         u32,
    /// Current retained-table frame number.
    frame:             u64,
    /// Whether the table has passed the frame's append window.
    frozen:            bool,
    /// Number of update attempts dropped because `row_limit` was reached.
    dropped_records:   u32,
    /// Sequential source key used only by raw-row unit tests.
    #[cfg(test)]
    test_append_index: u32,
}

impl Default for FrameMaterialTableBuilder {
    fn default() -> Self {
        Self {
            entries:                        Vec::new(),
            source_slots:                   HashMap::new(),
            retired:                        VecDeque::new(),
            rejected:                       HashSet::new(),
            new_assignments:                Vec::new(),
            row_limit:                      INVALID_GPU_MATERIAL_SLOT - 1,
            frame:                          0,
            frozen:                         false,
            dropped_records:                0,
            #[cfg(test)]
            test_append_index:              0,
        }
    }
}

impl FrameMaterialTableBuilder {
    /// Starts a new producer-update window for the current frame.
    pub(super) fn clear(&mut self, row_limit: u32) {
        self.frame = self.frame.saturating_add(1);
        self.row_limit = row_limit.min(INVALID_GPU_MATERIAL_SLOT - 1);
        self.trim_reusable_tail();
        self.rejected.clear();
        self.new_assignments.clear();
        self.frozen = false;
        self.dropped_records = 0;
        #[cfg(test)]
        {
            self.test_append_index = 0;
        }
    }

    /// Updates one source row and returns its retained slot id.
    pub(super) fn upsert_values(
        &mut self,
        source: MaterialSourceKey,
        values: MaterialSlotValues,
    ) -> FrameMaterialSlotAppend {
        assert!(
            !self.frozen,
            "FrameMaterialTableBuilder::upsert_values called after MaterialTableUpdatedToCurrent"
        );

        if let Some(slot) = self.source_slots.get(&source).copied() {
            let entry = &mut self.entries[slot.as_u32().to_usize()];
            entry.values = values;
            entry.seen_frame = self.frame;
            return FrameMaterialSlotAppend::Appended(slot);
        }

        let slot = self.take_reusable_slot().or_else(|| {
            (self.entries.len().to_u32() < self.row_limit).then(|| {
                let raw = self.entries.len().to_u32();
                assert_ne!(
                    raw, INVALID_GPU_MATERIAL_SLOT,
                    "FrameMaterialTableBuilder emitted the reserved GPU material-slot sentinel"
                );
                self.entries.push(MaterialSlotEntry::default());
                MaterialSlotId(raw)
            })
        });
        let Some(slot) = slot else {
            self.dropped_records = self.dropped_records.saturating_add(1);
            self.rejected.insert(source);
            return FrameMaterialSlotAppend::DroppedLimit;
        };
        let entry = &mut self.entries[slot.as_u32().to_usize()];
        *entry = MaterialSlotEntry {
            source: Some(source),
            values,
            seen_frame: self.frame,
        };
        self.source_slots.insert(source, slot);
        self.new_assignments.push((source, slot));
        FrameMaterialSlotAppend::Appended(slot)
    }

    #[cfg(test)]
    fn append_values(&mut self, values: MaterialSlotValues) -> FrameMaterialSlotAppend {
        let source = MaterialSourceKey::Test(self.test_append_index);
        self.test_append_index = self.test_append_index.saturating_add(1);
        self.upsert_values(source, values)
    }

    /// Marks the current new-assignment count for a producer transaction.
    #[must_use]
    pub(super) const fn assignment_checkpoint(&self) -> usize { self.new_assignments.len() }

    /// Returns the retained row extent.
    #[must_use]
    #[cfg(test)]
    pub(super) const fn row_count(&self) -> usize { self.entries.len() }

    /// Rolls back new source assignments made after `checkpoint`.
    pub(super) fn rollback_assignments_after(&mut self, checkpoint: usize) {
        for (source, slot) in self.new_assignments.drain(checkpoint..) {
            self.source_slots.remove(&source);
            self.entries[slot.as_u32().to_usize()] = MaterialSlotEntry::default();
            self.retired.push_back(RetiredMaterialSlot {
                slot,
                reusable_at_frame: self.frame,
            });
        }
        self.trim_reusable_tail();
    }

    /// Retires untouched sources and returns the extracted table payload.
    #[must_use]
    fn freeze(&mut self) -> FrameMaterialTable {
        assert!(
            !self.frozen,
            "FrameMaterialTableBuilder::freeze called more than once in one frame"
        );
        self.frozen = true;
        self.retire_unseen_sources();
        FrameMaterialTable {
            rows:      self.entries.iter().map(|entry| entry.values).collect(),
            live_rows: self.source_slots.len(),
        }
    }

    /// Returns the rows needed if every rejected new source retries next frame.
    #[must_use]
    fn required_row_count(&self) -> u32 {
        self.entries
            .len()
            .saturating_add(self.rejected.len())
            .to_u32()
    }

    /// Returns the number of rows dropped by the frame's row limit.
    #[must_use]
    pub(super) const fn dropped_record_count(&self) -> u32 { self.dropped_records }

    fn take_reusable_slot(&mut self) -> Option<MaterialSlotId> {
        let index = self.retired.iter().position(|retired| {
            retired.reusable_at_frame <= self.frame && retired.slot.as_u32() < self.row_limit
        })?;
        self.retired.remove(index).map(|retired| retired.slot)
    }

    fn retire_unseen_sources(&mut self) {
        let stale: Vec<_> = self
            .source_slots
            .iter()
            .filter_map(|(source, slot)| {
                (self.entries[slot.as_u32().to_usize()].seen_frame != self.frame)
                    .then_some((*source, *slot))
            })
            .collect();
        for (source, slot) in stale {
            self.source_slots.remove(&source);
            self.entries[slot.as_u32().to_usize()] = MaterialSlotEntry::default();
            self.retired.push_back(RetiredMaterialSlot {
                slot,
                reusable_at_frame: self.frame.saturating_add(MATERIAL_SLOT_RETIREMENT_FRAMES),
            });
        }
    }

    fn trim_reusable_tail(&mut self) {
        loop {
            let Some(last_index) = self.entries.len().checked_sub(1) else {
                return;
            };
            if self.entries[last_index].source.is_some() {
                return;
            }
            let raw = last_index.to_u32();
            let Some(retired_index) = self.retired.iter().position(|retired| {
                retired.slot.as_u32() == raw && retired.reusable_at_frame <= self.frame
            }) else {
                return;
            };
            self.retired.remove(retired_index);
            self.entries.pop();
        }
    }
}

/// Main-world owner for the frame table builder and frozen output.
#[derive(Debug, Reflect, Resource)]
#[reflect(Resource)]
pub(super) struct FrameMaterialTableBuild {
    /// Single builder shared by SDF, text, and panel-shape producers.
    #[reflect(ignore)]
    builder:     FrameMaterialTableBuilder,
    /// Frozen rows extracted with the frame's GPU records.
    table:       FrameMaterialTable,
    /// Storage-buffer-derived row cap for this frame.
    row_limit:   u32,
    /// Wall time of the most recent `freeze` clone.
    last_freeze: Duration,
    /// Wall time of the most recent capacity-sized table encoding.
    last_upload: Duration,
    /// Capacity-sized bytes extracted for the render-world queue write.
    #[reflect(ignore)]
    upload_data: Vec<u8>,
}

impl Default for FrameMaterialTableBuild {
    fn default() -> Self {
        Self {
            builder:     FrameMaterialTableBuilder::default(),
            table:       FrameMaterialTable::default(),
            row_limit:   INVALID_GPU_MATERIAL_SLOT - 1,
            last_freeze: Duration::ZERO,
            last_upload: Duration::ZERO,
            upload_data: Vec::new(),
        }
    }
}

impl FrameMaterialTableBuild {
    /// Starts the frame's append window using the configured row cap.
    fn clear(&mut self) { self.builder.clear(self.row_limit); }

    /// Starts a frame capped by the active GPU buffer while preserving the device cap.
    fn clear_with_active_capacity(&mut self, capacity: u32) {
        self.builder.clear(capacity.min(self.row_limit));
    }

    /// Updates the storage-buffer-derived row cap used by the next clear.
    fn set_row_limit(&mut self, row_limit: u32) {
        self.row_limit = row_limit.min(INVALID_GPU_MATERIAL_SLOT - 1);
    }

    /// Returns the single append builder for current-frame producers.
    pub(super) const fn builder_mut(&mut self) -> &mut FrameMaterialTableBuilder {
        &mut self.builder
    }

    /// Freezes the append builder and publishes `FrameMaterialTable`.
    fn freeze(&mut self) { self.table = self.builder.freeze(); }

    /// Returns the frozen frame table for extraction.
    #[must_use]
    pub(super) const fn table(&self) -> &FrameMaterialTable { &self.table }

    /// Maximum table capacity supported by the active render device.
    const fn row_limit(&self) -> u32 { self.row_limit }

    /// Returns the number of row-limit drops in the current frame.
    #[must_use]
    const fn dropped_record_count(&self) -> u32 { self.builder.dropped_record_count() }

    /// Returns the capacity needed when rejected sources retry.
    #[must_use]
    fn required_row_count(&self) -> u32 { self.builder.required_row_count() }

    fn encode_upload(&mut self, capacity: u32) {
        let start = Instant::now();
        self.upload_data = self.table.encoded_rows(capacity);
        self.last_upload = start.elapsed();
    }
}

fn material_table_capacity(required_rows: u32, row_limit: u32) -> u32 {
    let required_rows = required_rows.max(DEFAULT_TABLE_CAPACITY);
    let headroom = required_rows.div_ceil(CAPACITY_HEADROOM_DIVISOR);
    required_rows
        .saturating_add(headroom)
        .checked_next_power_of_two()
        .unwrap_or(u32::MAX)
        .min(row_limit.max(1))
}

const fn should_prepare_larger_buffer(required_rows: u32, capacity: u32) -> bool {
    required_rows >= capacity
}

#[derive(Clone, Debug)]
struct PendingMaterialTableBuffer {
    handle:   Handle<ShaderBuffer>,
    capacity: u32,
}

/// Main-world material-table storage-buffer handle and capacity.
#[derive(Clone, Debug, Default, Reflect, Resource)]
#[reflect(Resource)]
pub(super) struct MaterialTableBuffer {
    /// Shader buffer asset handle registered on batch materials.
    #[reflect(ignore)]
    pub handle:      Option<Handle<ShaderBuffer>>,
    /// Row capacity represented by `handle`.
    pub capacity:    u32,
    /// Buffer assets allocated for table growth events, since startup.
    pub allocations: LifetimeTotal,
    /// Table-buffer handle last rebound into all registered path materials.
    #[reflect(ignore)]
    bound_handle:    Option<Handle<ShaderBuffer>>,
    /// Larger buffer created after the activation boundary and bound there next frame.
    #[reflect(ignore)]
    pending:         Option<PendingMaterialTableBuffer>,
}

/// Render-world copy of the frame table and current buffer handle.
#[derive(Clone, Debug, Default, Resource)]
struct ExtractedFrameMaterialTable {
    /// Frozen rows extracted from `FrameMaterialTableBuild`.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Phase 3 record extraction reads material rows from this render-world resource"
        )
    )]
    table:  FrameMaterialTable,
    /// Current table buffer handle already rebound into batch materials.
    handle: Option<Handle<ShaderBuffer>>,
    /// Encoded capacity-sized payload written after `GpuShaderBuffer` preparation.
    data:   Vec<u8>,
}

/// One path batch material registered for material-table rebinding.
#[derive(Debug, Reflect)]
struct RegisteredPathMaterial {
    /// Batch material handle.
    #[reflect(ignore)]
    material:     Handle<PathExtendedMaterial>,
    /// Table-buffer handle last written to `PathExtension::material_table`.
    #[reflect(ignore)]
    bound_handle: Option<Handle<ShaderBuffer>>,
}

impl RegisteredPathMaterial {
    fn needs_rebind(&self, handle: &Handle<ShaderBuffer>) -> bool {
        self.bound_handle.as_ref() != Some(handle)
    }
}

/// One SDF batch material registered for material-table rebinding.
#[derive(Debug, Reflect)]
struct RegisteredSdfMaterial {
    /// Batch material handle.
    #[reflect(ignore)]
    material:     Handle<SdfExtendedMaterial>,
    /// Table-buffer handle last written to `SdfExtension::material_table`.
    #[reflect(ignore)]
    bound_handle: Option<Handle<ShaderBuffer>>,
}

impl RegisteredSdfMaterial {
    fn needs_rebind(&self, handle: &Handle<ShaderBuffer>) -> bool {
        self.bound_handle.as_ref() != Some(handle)
    }
}

/// Registered batch material handles that need table-buffer rebinding.
#[derive(Debug, Default, Reflect, Resource)]
#[reflect(Resource)]
pub(super) struct BatchMaterialTableRegistry {
    /// Path batch material handles keyed by their owning batch entity.
    path_materials: HashMap<Entity, RegisteredPathMaterial>,
    /// SDF batch material handles keyed by their owning batch entity.
    sdf_materials:  HashMap<Entity, RegisteredSdfMaterial>,
    /// At least one registered material has not seen the current table buffer.
    pending_rebind: bool,
}

impl BatchMaterialTableRegistry {
    /// Registers or replaces a path batch material for table-buffer rebinding.
    pub(super) fn register_path(
        &mut self,
        batch_entity: Entity,
        material: Handle<PathExtendedMaterial>,
    ) {
        match self.path_materials.get_mut(&batch_entity) {
            Some(registered) if registered.material == material => {},
            Some(registered) => {
                registered.material = material;
                registered.bound_handle = None;
                self.pending_rebind = true;
            },
            None => {
                self.path_materials.insert(
                    batch_entity,
                    RegisteredPathMaterial {
                        material,
                        bound_handle: None,
                    },
                );
                self.pending_rebind = true;
            },
        }
    }

    /// Registers or replaces an SDF batch material for table-buffer rebinding.
    fn register_sdf(&mut self, batch_entity: Entity, material: Handle<SdfExtendedMaterial>) {
        match self.sdf_materials.get_mut(&batch_entity) {
            Some(registered) if registered.material == material => {},
            Some(registered) => {
                registered.material = material;
                registered.bound_handle = None;
                self.pending_rebind = true;
            },
            None => {
                self.sdf_materials.insert(
                    batch_entity,
                    RegisteredSdfMaterial {
                        material,
                        bound_handle: None,
                    },
                );
                self.pending_rebind = true;
            },
        }
    }

    /// Explicitly removes a path batch material from table-buffer rebinding.
    pub(super) fn unregister_path(&mut self, batch_entity: Entity) {
        self.path_materials.remove(&batch_entity);
    }

    /// Removes registry entries whose batch entities no longer exist.
    fn purge_dead_with(&mut self, mut is_alive: impl FnMut(Entity) -> bool) {
        self.path_materials.retain(|entity, _| is_alive(*entity));
        self.sdf_materials.retain(|entity, _| is_alive(*entity));
    }

    /// Returns the number of registered batch material handles.
    #[must_use]
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Phase 2 registry cap tests use this diagnostic")
    )]
    fn len(&self) -> usize { self.path_materials.len() + self.sdf_materials.len() }

    /// Returns whether no batch material handles are registered.
    #[must_use]
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Phase 2 registry cap tests use this diagnostic")
    )]
    fn is_empty(&self) -> bool { self.path_materials.is_empty() && self.sdf_materials.is_empty() }

    fn needs_rebind(&self, handle: &Handle<ShaderBuffer>) -> bool {
        self.pending_rebind
            || self
                .path_materials
                .values()
                .any(|registered| registered.needs_rebind(handle))
            || self
                .sdf_materials
                .values()
                .any(|registered| registered.needs_rebind(handle))
    }
}

/// `PostUpdate` boundary where all batch material creation and registration finish.
#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct BatchResourcesReady;

/// Systems that create or replace retained batch assets.
///
/// Allocation and capacity growth run before Bevy's [`AssetEventSystems`] so
/// new meshes, buffers, and materials can enter extraction in the same frame.
/// Fixed-capacity record edits do not replace assets; their commit systems
/// stage direct writes through [`RetainedBatchBufferUploads`] instead.
#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct BatchAssetWrites;

/// `PostUpdate` boundary after which the frame material table is frozen.
///
/// A capacity-growth `ShaderBuffer` created in this set must remain in
/// `MaterialTableBuffer::pending` until the following frame. This set has no
/// ordering edge to `AssetEventSystems`, so binding a fresh handle here while
/// batch materials still reference the old buffer uploads nothing and renders
/// stale table rows. [`activate_prepared_material_table_buffer`] promotes the
/// pending handle in [`MaterialTableAppendReady`] on the following frame.
#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct MaterialTableUpdatedToCurrent;

/// `PostUpdate` boundary that clears the single frame table before producers append.
#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MaterialTableAppendReady;

/// Render-world queue writes for retained diegetic GPU buffers.
#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct DirectGpuBufferUploads;

#[derive(Clone, Debug)]
struct RetainedBatchBufferUpload {
    handle: Handle<ShaderBuffer>,
    data:   Vec<u8>,
}

/// Same-frame GPU writes staged by retained batch commit systems.
#[derive(Debug, Default, Resource)]
pub(super) struct RetainedBatchBufferUploads {
    uploads: Vec<RetainedBatchBufferUpload>,
}

impl RetainedBatchBufferUploads {
    fn clear(&mut self) { self.uploads.clear(); }

    fn stage(&mut self, handle: &Handle<ShaderBuffer>, data: Vec<u8>) {
        if let Some(upload) = self
            .uploads
            .iter_mut()
            .find(|upload| upload.handle.id() == handle.id())
        {
            upload.data = data;
            return;
        }
        self.uploads.push(RetainedBatchBufferUpload {
            handle: handle.clone(),
            data,
        });
    }

    #[cfg(test)]
    pub(super) fn contains(&self, handle: &Handle<ShaderBuffer>) -> bool {
        self.uploads
            .iter()
            .any(|upload| upload.handle.id() == handle.id())
    }

    #[cfg(test)]
    pub(super) fn data(&self, handle: &Handle<ShaderBuffer>) -> Option<&[u8]> {
        self.uploads
            .iter()
            .find(|upload| upload.handle.id() == handle.id())
            .map(|upload| upload.data.as_slice())
    }
}

/// Stages encoded fixed-capacity bytes for a retained batch's existing buffer.
pub(super) fn stage_retained_batch_buffer_upload(
    uploads: &mut RetainedBatchBufferUploads,
    handle: &Handle<ShaderBuffer>,
    encoded: ShaderBuffer,
) {
    let Some(data) = encoded.data else {
        return;
    };
    uploads.stage(handle, data);
}

#[derive(Clone, Debug, Default, Resource)]
struct ExtractedRetainedBatchBufferUploads {
    uploads: Vec<RetainedBatchBufferUpload>,
}

/// Plugin that wires the frame material table into main-world and render-world schedules.
pub(super) struct MaterialTablePlugin;

impl Plugin for MaterialTablePlugin {
    fn build(&self, app: &mut App) {
        debug_assert_binding_numbers_are_unique();
        debug_assert_material_slot_encoding();
        app.init_resource::<FrameMaterialTableBuild>()
            .init_resource::<MaterialTableBuffer>()
            .init_resource::<BatchMaterialTableRegistry>()
            .init_resource::<RetainedBatchBufferUploads>()
            .init_resource::<DiegeticPerfStats>()
            .configure_sets(
                PostUpdate,
                (
                    BatchAssetWrites.before(AssetEventSystems),
                    MaterialTableAppendReady.before(TransformSystems::Propagate),
                    BatchResourcesReady.after(TransformSystems::Propagate),
                    MaterialTableUpdatedToCurrent.after(BatchResourcesReady),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    activate_prepared_material_table_buffer,
                    clear_frame_material_table,
                    clear_retained_batch_buffer_uploads,
                )
                    .chain()
                    .in_set(MaterialTableAppendReady),
            )
            .add_systems(
                PostUpdate,
                purge_stale_registered_material_table_buffers.in_set(BatchResourcesReady),
            )
            .add_systems(
                PostUpdate,
                (
                    freeze_frame_material_table,
                    ensure_material_table_buffer_handle,
                    rebind_registered_material_table_buffers,
                    encode_material_table_upload,
                    update_material_table_perf_stats,
                    warn_material_table_drops,
                )
                    .chain()
                    .in_set(MaterialTableUpdatedToCurrent),
            );

        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .init_resource::<ExtractedFrameMaterialTable>()
                .init_resource::<ExtractedRetainedBatchBufferUploads>()
                .add_systems(
                    ExtractSchedule,
                    (
                        extract_frame_material_table,
                        extract_retained_batch_buffer_uploads,
                    ),
                )
                .add_systems(
                    Render,
                    (
                        upload_extracted_material_table,
                        upload_extracted_retained_batch_buffers,
                    )
                        .in_set(DirectGpuBufferUploads)
                        .in_set(RenderSystems::PrepareAssets),
                );
        }
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.configure_sets(
            Render,
            DirectGpuBufferUploads
                .after(prepare_assets::<GpuShaderBuffer>)
                .before(prepare_erased_assets::<MeshMaterial3d<SdfExtendedMaterial>>)
                .before(prepare_erased_assets::<MeshMaterial3d<ImageExtendedMaterial>>)
                .before(prepare_erased_assets::<MeshMaterial3d<PathExtendedMaterial>>),
        );
        let Some(max_bytes) = render_app
            .world()
            .get_resource::<RenderDevice>()
            .map(|render_device| render_device.limits().max_storage_buffer_binding_size)
        else {
            return;
        };
        set_material_table_row_limit_from_storage_buffer_bytes(app, max_bytes);
    }
}

fn activate_prepared_material_table_buffer(mut table_buffer: ResMut<MaterialTableBuffer>) {
    let Some(pending) = table_buffer.pending.take() else {
        return;
    };
    table_buffer.handle = Some(pending.handle);
    table_buffer.capacity = pending.capacity;
}

pub(super) fn clear_frame_material_table(
    mut build: ResMut<FrameMaterialTableBuild>,
    table_buffer: Res<MaterialTableBuffer>,
) {
    if table_buffer.handle.is_some() {
        build.clear_with_active_capacity(table_buffer.capacity);
    } else {
        build.clear();
    }
}

fn clear_retained_batch_buffer_uploads(mut uploads: ResMut<RetainedBatchBufferUploads>) {
    uploads.clear();
}

fn freeze_frame_material_table(mut build: ResMut<FrameMaterialTableBuild>) {
    let start = Instant::now();
    build.freeze();
    build.last_freeze = start.elapsed();
}

fn ensure_material_table_buffer_handle(
    build: Res<FrameMaterialTableBuild>,
    mut table_buffer: ResMut<MaterialTableBuffer>,
    mut storage_buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let required_rows = build
        .required_row_count()
        .max(build.table().row_count().to_u32());
    if table_buffer.handle.is_none() {
        let capacity = material_table_capacity(required_rows, build.row_limit());
        let shader_buffer = ShaderBuffer::from(build.table().padded_rows(capacity));
        table_buffer.handle = Some(storage_buffers.add(shader_buffer));
        table_buffer.capacity = capacity;
        table_buffer.allocations += 1;
        return;
    }
    if table_buffer.pending.is_some()
        || !should_prepare_larger_buffer(required_rows, table_buffer.capacity)
    {
        return;
    }
    let capacity = material_table_capacity(
        required_rows.max(table_buffer.capacity.saturating_add(1)),
        build.row_limit(),
    )
    .max(table_buffer.capacity)
    .min(build.row_limit());
    if capacity <= table_buffer.capacity {
        return;
    }
    let shader_buffer = ShaderBuffer::from(build.table().padded_rows(capacity));
    table_buffer.pending = Some(PendingMaterialTableBuffer {
        handle: storage_buffers.add(shader_buffer),
        capacity,
    });
    table_buffer.allocations += 1;
}

pub(super) fn register_path_batch_materials<T>(
    mut registry: ResMut<BatchMaterialTableRegistry>,
    batches: Query<
        (Entity, &MeshMaterial3d<PathExtendedMaterial>),
        (
            With<T>,
            Or<(Added<T>, Changed<MeshMaterial3d<PathExtendedMaterial>>)>,
        ),
    >,
) where
    T: Component,
{
    for (entity, material) in &batches {
        registry.register_path(entity, material.0.clone());
    }
}

pub(super) fn register_sdf_batch_materials<T>(
    #[cfg(test)] mut run_order: Option<ResMut<SdfDriverRunOrder>>,
    mut registry: ResMut<BatchMaterialTableRegistry>,
    batches: Query<
        (Entity, &MeshMaterial3d<SdfExtendedMaterial>),
        (
            With<T>,
            Or<(Added<T>, Changed<MeshMaterial3d<SdfExtendedMaterial>>)>,
        ),
    >,
) where
    T: Component,
{
    #[cfg(test)]
    let mut registered_batch_entity = false;
    for (entity, material) in &batches {
        registry.register_sdf(entity, material.0.clone());
        #[cfg(test)]
        {
            registered_batch_entity = true;
        }
    }
    #[cfg(test)]
    if let Some(run_order) = run_order.as_deref_mut() {
        run_order.names.push(SDF_DRIVER_REGISTER);
        run_order.registered_batch_entity |= registered_batch_entity;
    }
}

fn purge_stale_registered_material_table_buffers(
    mut registry: ResMut<BatchMaterialTableRegistry>,
    live_entities: Query<Entity>,
) {
    registry.purge_dead_with(|entity| live_entities.get(entity).is_ok());
}

fn rebind_registered_material_table_buffers(
    mut registry: ResMut<BatchMaterialTableRegistry>,
    mut table_buffer: ResMut<MaterialTableBuffer>,
    mut path_materials: ResMut<Assets<PathExtendedMaterial>>,
    sdf_materials: Option<ResMut<Assets<SdfExtendedMaterial>>>,
) {
    let Some(handle) = table_buffer.handle.clone() else {
        return;
    };
    if table_buffer.bound_handle.as_ref() == Some(&handle) && !registry.needs_rebind(&handle) {
        return;
    }
    rebind_registered_path_materials(&mut registry, &handle, &mut path_materials);
    if let Some(mut sdf_materials) = sdf_materials {
        rebind_registered_sdf_materials(&mut registry, &handle, &mut sdf_materials);
    }
    table_buffer.bound_handle = Some(handle);
}

fn rebind_registered_path_materials(
    registry: &mut BatchMaterialTableRegistry,
    handle: &Handle<ShaderBuffer>,
    path_materials: &mut Assets<PathExtendedMaterial>,
) -> usize {
    let mut rebound = 0;
    let mut pending_rebind = false;
    for registered in registry.path_materials.values_mut() {
        if !registered.needs_rebind(handle) {
            continue;
        }
        if let Some(mut material) = path_materials.get_mut(&registered.material) {
            super::set_path_material_table_buffer(&mut material, handle.clone());
            registered.bound_handle = Some(handle.clone());
            rebound += 1;
        } else {
            pending_rebind = true;
        }
    }
    registry.pending_rebind = pending_rebind;
    rebound
}

fn rebind_registered_sdf_materials(
    registry: &mut BatchMaterialTableRegistry,
    handle: &Handle<ShaderBuffer>,
    sdf_materials: &mut Assets<SdfExtendedMaterial>,
) -> usize {
    let mut rebound = 0;
    let mut pending_rebind = false;
    for registered in registry.sdf_materials.values_mut() {
        if !registered.needs_rebind(handle) {
            continue;
        }
        if let Some(mut material) = sdf_materials.get_mut(&registered.material) {
            super::set_sdf_material_table_buffer(&mut material, handle.clone());
            registered.bound_handle = Some(handle.clone());
            rebound += 1;
        } else {
            pending_rebind = true;
        }
    }
    registry.pending_rebind |= pending_rebind;
    rebound
}

fn encode_material_table_upload(
    mut build: ResMut<FrameMaterialTableBuild>,
    table_buffer: Res<MaterialTableBuffer>,
) {
    build.last_upload = Duration::ZERO;
    if table_buffer.handle.is_none() {
        return;
    }
    let capacity = table_buffer.capacity.max(DEFAULT_TABLE_CAPACITY);
    build.encode_upload(capacity);
}

fn update_material_table_perf_stats(
    build: Res<FrameMaterialTableBuild>,
    table_buffer: Res<MaterialTableBuffer>,
    mut perf: ResMut<DiegeticPerfStats>,
) {
    perf.material_table.rows = build.table().live_row_count().into();
    perf.material_table.upload_bytes = build.upload_data.len().into();
    perf.material_table.capacity = table_buffer.capacity.to_usize().into();
    perf.material_table.freeze_us =
        u64::try_from(build.last_freeze.as_micros()).unwrap_or(u64::MAX);
    perf.material_table.upload_us =
        u64::try_from(build.last_upload.as_micros()).unwrap_or(u64::MAX);
    perf.material_table.allocations = table_buffer.allocations;
}

fn warn_material_table_drops(build: Res<FrameMaterialTableBuild>) {
    let drops = build.dropped_record_count();
    if drops > 0 {
        warn!(
            material_table.dropped_records = drops,
            "material table row limit dropped render records this frame"
        );
    }
}

fn extract_frame_material_table(
    mut commands: Commands,
    build: Extract<Res<FrameMaterialTableBuild>>,
    table_buffer: Extract<Res<MaterialTableBuffer>>,
) {
    commands.insert_resource(ExtractedFrameMaterialTable {
        table:  build.table().clone(),
        handle: table_buffer.handle.clone(),
        data:   build.upload_data.clone(),
    });
}

fn extract_retained_batch_buffer_uploads(
    mut commands: Commands,
    uploads: Extract<Res<RetainedBatchBufferUploads>>,
) {
    commands.insert_resource(ExtractedRetainedBatchBufferUploads {
        uploads: uploads.uploads.clone(),
    });
}

fn upload_extracted_material_table(
    extracted: Res<ExtractedFrameMaterialTable>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
    render_queue: Res<RenderQueue>,
) {
    let Some(handle) = extracted.handle.as_ref() else {
        return;
    };
    let Some(gpu_buffer) = gpu_buffers.get(handle) else {
        return;
    };
    if extracted.data.is_empty() {
        return;
    }
    render_queue.write_buffer(&gpu_buffer.buffer, 0, &extracted.data);
}

fn upload_extracted_retained_batch_buffers(
    extracted: Res<ExtractedRetainedBatchBufferUploads>,
    gpu_buffers: Res<RenderAssets<GpuShaderBuffer>>,
    render_queue: Res<RenderQueue>,
) {
    for upload in &extracted.uploads {
        let Some(gpu_buffer) = gpu_buffers.get(&upload.handle) else {
            continue;
        };
        if upload.data.len() as u64 != gpu_buffer.buffer_descriptor.size {
            warn_once!(
                "retained batch upload size {} did not match GPU buffer size {} for {:?}",
                upload.data.len(),
                gpu_buffer.buffer_descriptor.size,
                upload.handle.id(),
            );
            continue;
        }
        render_queue.write_buffer(&gpu_buffer.buffer, 0, &upload.data);
    }
}

fn debug_assert_binding_numbers_are_unique() {
    let bindings = [
        PATH_UNIFORM_BINDING,
        PATH_CURVES_BINDING,
        PATH_BANDS_BINDING,
        PATH_RECORDS_BINDING,
        PATH_INSTANCES_BINDING,
        PATH_RUN_RECORDS_BINDING,
        MATERIAL_TABLE_BINDING,
        SDF_RENDER_RECORDS_BINDING,
        SDF_MESH_BINDING,
    ];
    for (index, binding) in bindings.iter().enumerate() {
        debug_assert!(
            !bindings[index + 1..].contains(binding),
            "duplicate diegetic material binding {binding}"
        );
    }
}

fn debug_assert_material_slot_encoding() {
    let slot = MaterialSlotId(0);
    let authored = SdfPaintMaterial::Authored(slot).to_gpu();
    let absent = SdfPaintMaterial::NotAuthored.to_gpu();
    debug_assert_ne!(authored.as_u32(), INVALID_GPU_MATERIAL_SLOT);
    debug_assert_eq!(absent.as_u32(), INVALID_GPU_MATERIAL_SLOT);
    debug_assert_eq!(
        SdfPaintMaterial::from_gpu(authored, 1),
        Ok(SdfPaintMaterial::Authored(slot))
    );
    debug_assert_eq!(
        SdfPaintMaterial::from_gpu(absent, 1),
        Ok(SdfPaintMaterial::NotAuthored)
    );
}

fn material_row_limit_from_storage_buffer_bytes(max_bytes: u64) -> u32 {
    let row_size = u64::try_from(MaterialSlotValues::shader_size_bytes()).unwrap_or(u64::MAX);
    let rows = max_bytes / row_size.max(1);
    u32::try_from(rows)
        .unwrap_or(INVALID_GPU_MATERIAL_SLOT - 1)
        .min(INVALID_GPU_MATERIAL_SLOT - 1)
}

fn set_material_table_row_limit_from_storage_buffer_bytes(app: &mut App, max_bytes: u64) {
    let row_limit = material_row_limit_from_storage_buffer_bytes(max_bytes);
    app.world_mut()
        .resource_mut::<FrameMaterialTableBuild>()
        .set_row_limit(row_limit);
}

/// One emitted measurement row from the synthetic material-table harness.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct MaterialTableMeasurement {
    /// Scenario name emitted with the structured measurement line.
    pub scenario:                &'static str,
    /// Number of sampled frames after warmup.
    pub sampled_frames:          usize,
    /// Number of material entries appended by the scenario.
    pub rows:                    usize,
    /// Number of bytes represented by the live rows.
    pub upload_bytes:            usize,
    /// Builder vector capacity after the scenario completes.
    pub capacity:                usize,
    /// Number of table-buffer allocations simulated by capacity growth.
    pub allocations:             u32,
    /// Total table-build time across sampled frames.
    pub build_time:              Duration,
    /// Material-id refresh time for animation-only text-run updates.
    pub material_refresh_bucket: Duration,
}

/// Runs the Phase 2 synthetic material-table measurements and prints structured rows.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Phase 2 exposes this harness for the Phase 2 to Phase 3 decision"
    )
)]
fn emit_material_table_measurements() -> Vec<MaterialTableMeasurement> {
    let measurements = vec![
        measure_material_table_scenario("small_mixed", SMALL_MEASUREMENT_ENTRIES),
        measure_material_table_scenario("medium_mixed", MEDIUM_MEASUREMENT_ENTRIES),
        measure_material_table_scenario("stress_mixed", STRESS_MEASUREMENT_ENTRIES),
    ];
    for measurement in &measurements {
        info!(
            material_table.scenario = measurement.scenario,
            material_table.sampled_frames = measurement.sampled_frames,
            material_table.rows = measurement.rows,
            material_table.upload_bytes = measurement.upload_bytes,
            material_table.capacity = measurement.capacity,
            material_table.allocations = measurement.allocations,
            material_table.build_time_us = measurement.build_time.as_micros(),
            material_table.material_refresh_us = measurement.material_refresh_bucket.as_micros(),
            "material table measurement"
        );
        println!(
            "material_table_measurement scenario={} sampled_frames={} rows={} upload_bytes={} \
             capacity={} allocations={} build_time_us={} material_refresh_us={}",
            measurement.scenario,
            measurement.sampled_frames,
            measurement.rows,
            measurement.upload_bytes,
            measurement.capacity,
            measurement.allocations,
            measurement.build_time.as_micros(),
            measurement.material_refresh_bucket.as_micros()
        );
    }
    measurements
}

fn measure_material_table_scenario(
    scenario: &'static str,
    entries: usize,
) -> MaterialTableMeasurement {
    let mut builder = FrameMaterialTableBuilder::default();
    let materials = synthetic_materials(entries);
    let mut build_time = Duration::ZERO;
    let mut material_refresh_bucket = Duration::ZERO;
    let mut allocations = 0_u32;
    let mut observed_capacity = 0_usize;

    for frame in 0..(MATERIAL_TABLE_WARMUP_FRAMES + MATERIAL_TABLE_STRESS_FRAMES) {
        let churned_entries = entries.saturating_sub(entries * TOPOLOGY_CHURN_PERCENT / 100);
        let live_entries = if frame % 2 == 0 {
            entries
        } else {
            churned_entries
        };
        builder.clear(INVALID_GPU_MATERIAL_SLOT - 1);
        let build_start = Instant::now();
        for (index, material) in materials.iter().take(live_entries).enumerate() {
            let _ = builder.upsert_values(
                MaterialSourceKey::Measurement(index.to_u32()),
                MaterialSlotValues::from(material),
            );
        }
        let table = builder.freeze();
        let elapsed = build_start.elapsed();
        if frame >= MATERIAL_TABLE_WARMUP_FRAMES {
            build_time += elapsed;
        }
        if table.capacity() > observed_capacity {
            observed_capacity = table.capacity();
            allocations = allocations.saturating_add(1);
        }

        builder.clear(INVALID_GPU_MATERIAL_SLOT - 1);
        let refresh_start = Instant::now();
        for (index, material) in materials.iter().take(live_entries).enumerate() {
            let mut animated = material.clone();
            animated.base_color = synthetic_color(index + frame);
            let _ = builder.upsert_values(
                MaterialSourceKey::Measurement(index.to_u32()),
                MaterialSlotValues::from(&animated),
            );
        }
        let _ = builder.freeze();
        if frame >= MATERIAL_TABLE_WARMUP_FRAMES {
            material_refresh_bucket += refresh_start.elapsed();
        }
    }

    let rows = entries;
    MaterialTableMeasurement {
        scenario,
        sampled_frames: MATERIAL_TABLE_STRESS_FRAMES,
        rows,
        upload_bytes: rows.saturating_mul(MaterialSlotValues::shader_size_bytes()),
        capacity: observed_capacity,
        allocations,
        build_time,
        material_refresh_bucket,
    }
}

fn synthetic_materials(entries: usize) -> Vec<StandardMaterial> {
    (0..entries)
        .map(|index| StandardMaterial {
            base_color: synthetic_color(index),
            emissive: LinearRgba::rgb(
                (index % 7).to_f32() * 0.01,
                (index % 11).to_f32() * 0.01,
                (index % 13).to_f32() * 0.01,
            ),
            perceptual_roughness: (index % 5).to_f32().mul_add(0.1, 0.2),
            metallic: (index % 3).to_f32() * 0.1,
            reflectance: (index % 4).to_f32().mul_add(0.1, 0.3),
            clearcoat: (index % 2).to_f32() * 0.1,
            anisotropy_strength: (index % 6).to_f32() * 0.05,
            anisotropy_rotation: (index % 8).to_f32() * 0.2,
            ..default()
        })
        .collect()
}

fn synthetic_color(index: usize) -> Color {
    let red = (index % 17).to_f32() / 17.0;
    let green = (index % 19).to_f32() / 19.0;
    let blue = (index % 23).to_f32() / 23.0;
    Color::srgb(red, green, blue)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected fixture setup"
)]
mod tests {
    use bevy::app::SubApp;
    use bevy::asset::AssetPlugin;
    use bevy::camera::visibility::VisibilityPlugin;
    use bevy::ecs::schedule::NodeId;
    use bevy::ecs::schedule::Schedule;
    use bevy::ecs::schedule::Schedules;
    use bevy::ecs::schedule::SystemSet;
    use bevy::math::Affine2;
    use bevy::pbr::MaterialPlugin;
    use bevy::prelude::AlphaMode;
    use bevy::prelude::IntoSystemSet;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Update;
    use bevy::prelude::World;
    use bevy::render::render_resource::Face;
    use bevy::render::render_resource::ShaderType;

    use super::*;
    use crate::render;
    use crate::render::AntiAlias;
    use crate::render::PathAtlasHandles;
    use crate::render::PathMaterialBuffers;
    use crate::render::RenderMode;
    use crate::render::batch_key::BatchAlphaMode;
    use crate::render::draw_order::ElementIndex;
    use crate::render::fill_batch::FillBatchPlugin;
    use crate::render::image_batch::ImageBatchPlugin;
    use crate::render::panel_shapes::PanelShapePlugin;
    use crate::render::panel_text::TextRenderPlugin;

    const MATERIAL_SLOT_VALUES_SHADER_SIZE_BYTES: u64 = 160;
    const STANDARD_MATERIAL_UNIFORM_SHADER_SIZE_BYTES: u64 = 192;
    /// Measured at 131 live rows in the `widgets` example on 2026-07-31; used here as a
    /// realistic probe size.
    const PROBE_SOURCE_COUNT: u32 = 131;
    const MATERIAL_SLOT_VALUES_WGSL_FIELDS: [&str; 16] = [
        "base_color",
        "emissive",
        "attenuation_color",
        "uv_transform",
        "reflectance",
        "roughness",
        "metallic",
        "diffuse_transmission",
        "specular_transmission",
        "thickness",
        "ior",
        "attenuation_distance",
        "clearcoat",
        "clearcoat_perceptual_roughness",
        "anisotropy_strength",
        "anisotropy_rotation",
    ];

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum ProducerKey {
        Sdf(u32),
        Text(u32),
        Shape(u32),
    }

    impl From<ProducerKey> for MaterialSourceKey {
        fn from(key: ProducerKey) -> Self {
            match key {
                ProducerKey::Sdf(index) => Self::TestSdf(index),
                ProducerKey::Text(index) => Self::TestText(index),
                ProducerKey::Shape(index) => Self::TestPanelLine(index),
            }
        }
    }

    struct TestInput {
        key:      ProducerKey,
        material: StandardMaterial,
    }

    #[derive(Resource)]
    struct TestMaterialRows(u32);

    impl MaterialSlotInput for TestInput {
        type Key = ProducerKey;

        fn key(&self) -> Self::Key { self.key }

        fn material_slot_candidate(&self) -> MaterialSlotCandidate {
            MaterialSlotCandidate::from(&self.material)
        }
    }

    fn write_test_material_rows(
        rows: Res<TestMaterialRows>,
        mut build: ResMut<FrameMaterialTableBuild>,
    ) {
        let values = MaterialSlotValues::from(&StandardMaterial::default());
        for index in 0..rows.0 {
            let _ = build
                .builder_mut()
                .upsert_values(MaterialSourceKey::Test(index), values);
        }
    }

    fn assert_material_table_frame(app: &App, expected_live_rows: u32) {
        let build = app.world().resource::<FrameMaterialTableBuild>();
        assert_eq!(
            build.table().live_row_count(),
            expected_live_rows.to_usize()
        );
        assert_eq!(build.dropped_record_count(), 0);
    }

    fn assert_material_table_rekey_frame(app: &App, expected_live_rows: u32) {
        assert_material_table_frame(app, expected_live_rows);
        let build = app.world().resource::<FrameMaterialTableBuild>();
        assert_eq!(
            build.table().row_count(),
            expected_live_rows.saturating_mul(2).to_usize()
        );
    }

    fn material_table_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Assets::<ShaderBuffer>::default())
            .init_resource::<FrameMaterialTableBuild>()
            .init_resource::<MaterialTableBuffer>();
        app
    }

    fn material_with_values(index: usize) -> StandardMaterial {
        StandardMaterial {
            base_color: Color::srgb(index.to_f32().mul_add(0.01, 0.1), 0.2, 0.3),
            emissive: LinearRgba::rgb(index.to_f32() * 0.02, 0.1, 0.2),
            emissive_exposure_weight: 0.4,
            perceptual_roughness: 0.7,
            metallic: 0.3,
            reflectance: 0.8,
            specular_tint: Color::srgb(0.25, 0.5, 0.75),
            diffuse_transmission: 0.1,
            specular_transmission: 0.2,
            thickness: 0.3,
            ior: 1.4,
            attenuation_distance: 2.0,
            attenuation_color: Color::srgb(0.7, 0.8, 0.9),
            clearcoat: 0.15,
            clearcoat_perceptual_roughness: 0.35,
            anisotropy_strength: 0.45,
            anisotropy_rotation: 0.6,
            uv_transform: Affine2::from_scale_angle_translation(
                Vec2::new(2.0, 3.0),
                0.25,
                Vec2::new(0.1, 0.2),
            ),
            ..default()
        }
    }

    fn test_path_material() -> PathExtendedMaterial {
        let atlas = PathAtlasHandles {
            curves:       Handle::default(),
            bands:        Handle::default(),
            path_records: Handle::default(),
        };
        PathExtendedMaterial {
            base:      StandardMaterial::default(),
            extension: render::analytic_paths::vertex_pull(
                RenderMode::Text,
                0.0,
                AntiAlias::default(),
                PathMaterialBuffers {
                    curves:       atlas.curves,
                    bands:        atlas.bands,
                    path_records: atlas.path_records,
                    instances:    Handle::default(),
                    run_records:  Handle::default(),
                },
            ),
        }
    }

    fn assert_same_f32(actual: f32, expected: f32) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }

    fn with_initialized_post_update<T>(app: &mut App, inspect: impl FnOnce(&Schedule) -> T) -> T {
        let mut schedules = app
            .world_mut()
            .remove_resource::<Schedules>()
            .expect("Schedules resource should exist");
        let result = {
            let world = app.world_mut();
            let schedule = schedules
                .get_mut(PostUpdate)
                .expect("PostUpdate schedule should exist");
            schedule
                .initialize(world)
                .expect("PostUpdate schedule should initialize");
            inspect(schedule)
        };
        app.world_mut().insert_resource(schedules);
        result
    }

    fn with_initialized_render<T>(app: &mut App, inspect: impl FnOnce(&Schedule) -> T) -> T {
        let render_app = app
            .get_sub_app_mut(RenderApp)
            .expect("the fixture should include a render app");
        let mut schedules = render_app
            .world_mut()
            .remove_resource::<Schedules>()
            .expect("the render app should own schedules");
        let result = {
            let world = render_app.world_mut();
            let schedule = schedules
                .get_mut(Render)
                .expect("the Render schedule should exist");
            schedule
                .initialize(world)
                .expect("the Render schedule should initialize");
            inspect(schedule)
        };
        render_app.world_mut().insert_resource(schedules);
        result
    }

    struct BatchScheduleSnapshot {
        append_ready: Vec<String>,
        resources:    Vec<String>,
        asset_writes: Vec<String>,
    }

    fn inspect_batch_schedule(schedule: &Schedule) -> BatchScheduleSnapshot {
        let update_text = schedule
            .systems()
            .expect("PostUpdate schedule should be initialized")
            .find_map(|(system_key, system)| {
                system
                    .name()
                    .contains("update_panel_text_batches")
                    .then_some(system_key)
            })
            .expect("update_panel_text_batches should be present in PostUpdate");
        let graph = schedule.graph();
        let batch_set = graph
            .system_sets
            .get_key(BatchResourcesReady.intern())
            .expect("BatchResourcesReady set should exist");
        let material_set = graph
            .system_sets
            .get_key(MaterialTableUpdatedToCurrent.intern())
            .expect("MaterialTableUpdatedToCurrent set should exist");
        let asset_event_set = graph
            .system_sets
            .get_key(AssetEventSystems.intern())
            .expect("AssetEventSystems set should exist");
        let asset_write_set = graph
            .system_sets
            .get_key(BatchAssetWrites.intern())
            .expect("BatchAssetWrites set should exist");
        assert!(
            graph
                .dependency()
                .contains_edge(NodeId::Set(batch_set), NodeId::Set(material_set))
        );
        assert!(
            graph
                .dependency()
                .contains_edge(NodeId::System(update_text), NodeId::Set(batch_set))
        );
        assert!(
            graph
                .dependency()
                .contains_edge(NodeId::Set(asset_write_set), NodeId::Set(asset_event_set)),
            "retained batch asset writes must run before Bevy AssetEventSystems"
        );
        let batch_systems = graph
            .systems_in_set(BatchResourcesReady.intern())
            .expect("BatchResourcesReady systems should resolve");
        let asset_write_systems = graph
            .systems_in_set(BatchAssetWrites.intern())
            .expect("BatchAssetWrites systems should resolve");
        let append_ready_systems = graph
            .systems_in_set(MaterialTableAppendReady.intern())
            .expect("MaterialTableAppendReady systems should resolve");
        let resources = schedule
            .systems()
            .expect("PostUpdate schedule should be initialized")
            .filter(|(system_key, _)| batch_systems.contains(system_key))
            .map(|(_, system)| system.name().to_string())
            .collect();
        let asset_writes = schedule
            .systems()
            .expect("PostUpdate schedule should be initialized")
            .filter(|(system_key, _)| asset_write_systems.contains(system_key))
            .map(|(_, system)| system.name().to_string())
            .collect();
        let append_ready = schedule
            .systems()
            .expect("PostUpdate schedule should be initialized")
            .filter(|(system_key, _)| append_ready_systems.contains(system_key))
            .map(|(_, system)| system.name().to_string())
            .collect();
        BatchScheduleSnapshot {
            append_ready,
            resources,
            asset_writes,
        }
    }

    struct DirectUploadScheduleSnapshot {
        systems:                     Vec<String>,
        all_in_set:                  bool,
        after_buffer_preparation:    bool,
        before_material_preparation: bool,
    }

    fn inspect_direct_upload_schedule(schedule: &Schedule) -> DirectUploadScheduleSnapshot {
        let graph = schedule.graph();
        let buffer_set = graph
            .system_sets
            .get_key(prepare_assets::<GpuShaderBuffer>.into_system_set().intern())
            .expect("GpuShaderBuffer preparation should exist");
        let upload_set = graph
            .system_sets
            .get_key(DirectGpuBufferUploads.intern())
            .expect("DirectGpuBufferUploads set should exist");
        let uploads = schedule
            .systems()
            .expect("the Render schedule should be initialized")
            .filter_map(|(system_key, system)| {
                (system.name().contains("upload_extracted_material_table")
                    || system
                        .name()
                        .contains("upload_extracted_retained_batch_buffers"))
                .then_some((system_key, system.name().to_string()))
            })
            .collect::<Vec<_>>();
        let material_sets = [
            graph
                .system_sets
                .get_key(
                    prepare_erased_assets::<MeshMaterial3d<SdfExtendedMaterial>>
                        .into_system_set()
                        .intern(),
                )
                .expect("SDF material preparation should exist"),
            graph
                .system_sets
                .get_key(
                    prepare_erased_assets::<MeshMaterial3d<ImageExtendedMaterial>>
                        .into_system_set()
                        .intern(),
                )
                .expect("image material preparation should exist"),
            graph
                .system_sets
                .get_key(
                    prepare_erased_assets::<MeshMaterial3d<PathExtendedMaterial>>
                        .into_system_set()
                        .intern(),
                )
                .expect("path material preparation should exist"),
        ];
        let upload_systems = graph
            .systems_in_set(DirectGpuBufferUploads.intern())
            .expect("DirectGpuBufferUploads systems should resolve");
        DirectUploadScheduleSnapshot {
            systems:                     uploads.iter().map(|(_, name)| name.clone()).collect(),
            all_in_set:                  uploads
                .iter()
                .all(|(system, _)| upload_systems.contains(system)),
            after_buffer_preparation:    graph
                .dependency()
                .contains_edge(NodeId::Set(buffer_set), NodeId::Set(upload_set)),
            before_material_preparation: material_sets.iter().all(|material_set| {
                graph
                    .dependency()
                    .contains_edge(NodeId::Set(upload_set), NodeId::Set(*material_set))
            }),
        }
    }

    fn wgsl_material_slot_fields(wgsl: &str) -> Vec<&str> {
        let Some((_, after_start)) = wgsl.split_once("struct MaterialSlotValues {") else {
            return Vec::new();
        };
        let Some((body, _)) = after_start.split_once('}') else {
            return Vec::new();
        };
        body.lines()
            .filter_map(|line| line.trim().split_once(':').map(|(name, _)| name.trim()))
            .collect()
    }

    #[test]
    fn material_slot_id_rejects_reserved_sentinel_and_out_of_range_rows() {
        assert_eq!(
            MaterialSlotId::try_from(INVALID_GPU_MATERIAL_SLOT),
            Err(MaterialSlotIdError::ReservedSentinel)
        );
        assert_eq!(
            MaterialSlotId::from_raw_in_table(INVALID_GPU_MATERIAL_SLOT, 1),
            Err(MaterialSlotIdError::ReservedSentinel)
        );
        assert_eq!(
            MaterialSlotId::from_raw_in_table(3, 2),
            Err(MaterialSlotIdError::OutOfRange {
                raw:       3,
                row_count: 2,
            })
        );
        assert_eq!(
            MaterialSlotId::from_raw_in_table(0, 1),
            Ok(MaterialSlotId(0))
        );
    }

    #[test]
    fn sdf_paint_material_is_the_only_gpu_sentinel_source() {
        let slot_zero = MaterialSlotId(0);
        let authored = SdfPaintMaterial::Authored(slot_zero).to_gpu();
        let not_authored = SdfPaintMaterial::NotAuthored.to_gpu();

        assert_eq!(authored.as_u32(), 0);
        assert_ne!(authored.as_u32(), INVALID_GPU_MATERIAL_SLOT);
        assert_eq!(not_authored.as_u32(), INVALID_GPU_MATERIAL_SLOT);
        assert_eq!(
            SdfPaintMaterial::from_gpu(authored, 1),
            Ok(SdfPaintMaterial::Authored(slot_zero))
        );
        assert_eq!(
            SdfPaintMaterial::from_gpu(not_authored, 1),
            Ok(SdfPaintMaterial::NotAuthored)
        );
    }

    #[test]
    fn new_sources_receive_distinct_rows_in_append_order() {
        let mut builder = FrameMaterialTableBuilder::default();
        builder.clear(INVALID_GPU_MATERIAL_SLOT - 1);
        let values = MaterialSlotValues::from(&StandardMaterial::default());

        let first = builder.append_values(values);
        let second = builder.append_values(values);
        let third = builder.append_values(values);

        assert_eq!(first, FrameMaterialSlotAppend::Appended(MaterialSlotId(0)));
        assert_eq!(second, FrameMaterialSlotAppend::Appended(MaterialSlotId(1)));
        assert_eq!(third, FrameMaterialSlotAppend::Appended(MaterialSlotId(2)));
        let table = builder.freeze();
        assert_eq!(table.row_count(), 3);
        assert_eq!(table.rows(), &[values, values, values]);
    }

    #[test]
    fn retained_sources_keep_slots_when_producer_order_changes() {
        let mut builder = FrameMaterialTableBuilder::default();
        let first_source = MaterialSourceKey::TestSdf(1);
        let second_source = MaterialSourceKey::TestText(2);
        let first_values = MaterialSlotValues::from(&material_with_values(1));
        let second_values = MaterialSlotValues::from(&material_with_values(2));

        builder.clear(INVALID_GPU_MATERIAL_SLOT - 1);
        let first_slot = builder.upsert_values(first_source, first_values);
        let second_slot = builder.upsert_values(second_source, second_values);
        let _ = builder.freeze();

        builder.clear(INVALID_GPU_MATERIAL_SLOT - 1);
        let second_again = builder.upsert_values(second_source, second_values);
        let first_again = builder.upsert_values(first_source, first_values);
        let table = builder.freeze();

        assert_eq!(first_again, first_slot);
        assert_eq!(second_again, second_slot);
        assert_eq!(table.rows()[0], first_values);
        assert_eq!(table.rows()[1], second_values);
    }

    #[test]
    fn retired_slot_waits_two_frames_before_reuse() {
        let mut builder = FrameMaterialTableBuilder::default();
        let removed_source = MaterialSourceKey::TestSdf(1);
        let retained_source = MaterialSourceKey::TestText(2);
        let next_source = MaterialSourceKey::TestPanelLine(3);
        let later_source = MaterialSourceKey::TestSdf(4);
        let values = MaterialSlotValues::from(&StandardMaterial::default());

        builder.clear(INVALID_GPU_MATERIAL_SLOT - 1);
        let removed_slot = builder.upsert_values(removed_source, values);
        let _ = builder.upsert_values(retained_source, values);
        let _ = builder.freeze();

        builder.clear(INVALID_GPU_MATERIAL_SLOT - 1);
        let _ = builder.upsert_values(retained_source, values);
        let _ = builder.freeze();

        builder.clear(INVALID_GPU_MATERIAL_SLOT - 1);
        let _ = builder.upsert_values(retained_source, values);
        let next_slot = builder.upsert_values(next_source, values);
        let _ = builder.freeze();
        assert_ne!(next_slot, removed_slot);

        builder.clear(INVALID_GPU_MATERIAL_SLOT - 1);
        let _ = builder.upsert_values(retained_source, values);
        let _ = builder.upsert_values(next_source, values);
        let later_slot = builder.upsert_values(later_source, values);
        let _ = builder.freeze();
        assert_eq!(later_slot, removed_slot);
    }

    #[test]
    fn existing_source_updates_when_the_table_is_at_capacity() {
        let mut builder = FrameMaterialTableBuilder::default();
        let source = MaterialSourceKey::TestSdf(1);
        let first_values = MaterialSlotValues::from(&material_with_values(1));
        let second_values = MaterialSlotValues::from(&material_with_values(2));

        builder.clear(1);
        let first_slot = builder.upsert_values(source, first_values);
        let _ = builder.freeze();

        builder.clear(1);
        let second_slot = builder.upsert_values(source, second_values);
        let table = builder.freeze();

        assert_eq!(second_slot, first_slot);
        assert_eq!(table.rows(), &[second_values]);
        assert_eq!(builder.dropped_record_count(), 0);
    }

    #[test]
    fn rollback_returns_a_reused_slot_to_the_current_frame() {
        let mut builder = FrameMaterialTableBuilder::default();
        let removed_source = MaterialSourceKey::TestSdf(1);
        let retained_source = MaterialSourceKey::TestText(2);
        let rolled_back_source = MaterialSourceKey::TestPanelLine(3);
        let replacement_source = MaterialSourceKey::TestSdf(4);
        let values = MaterialSlotValues::from(&StandardMaterial::default());

        builder.clear(2);
        let removed_slot = builder.upsert_values(removed_source, values);
        let _ = builder.upsert_values(retained_source, values);
        let _ = builder.freeze();

        for _ in 0..2 {
            builder.clear(2);
            let _ = builder.upsert_values(retained_source, values);
            let _ = builder.freeze();
        }

        builder.clear(2);
        let _ = builder.upsert_values(retained_source, values);
        let checkpoint = builder.assignment_checkpoint();
        let rolled_back_slot = builder.upsert_values(rolled_back_source, values);
        assert_eq!(rolled_back_slot, removed_slot);
        builder.rollback_assignments_after(checkpoint);

        let replacement_slot = builder.upsert_values(replacement_source, values);
        assert_eq!(replacement_slot, removed_slot);
        assert!(!builder.source_slots.contains_key(&rolled_back_source));
    }

    #[test]
    fn capacity_includes_headroom_and_rounds_to_a_power_of_two() {
        assert_eq!(
            material_table_capacity(343, INVALID_GPU_MATERIAL_SLOT - 1),
            1024
        );
        assert_eq!(
            material_table_capacity(600, INVALID_GPU_MATERIAL_SLOT - 1),
            2048
        );
    }

    #[test]
    fn capacity_keeps_headroom_below_a_power_of_two_boundary() {
        assert_eq!(
            material_table_capacity(126, INVALID_GPU_MATERIAL_SLOT - 1),
            256
        );
    }

    #[test]
    fn capacity_respects_the_render_device_row_limit() {
        assert_eq!(material_table_capacity(600, 700), 700);
    }

    #[test]
    fn spare_capacity_does_not_prepare_a_replacement_buffer() {
        assert!(!should_prepare_larger_buffer(255, 256));
        assert!(should_prepare_larger_buffer(256, 256));
    }

    #[test]
    fn growth_is_required_when_demand_exceeds_capacity() {
        assert!(should_prepare_larger_buffer(257, 256));
    }

    #[test]
    fn larger_buffer_activates_at_the_next_frame_boundary() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Assets::<ShaderBuffer>::default())
            .init_resource::<FrameMaterialTableBuild>()
            .init_resource::<MaterialTableBuffer>()
            .insert_resource(TestMaterialRows(1))
            .add_systems(
                Update,
                (
                    activate_prepared_material_table_buffer,
                    clear_frame_material_table,
                    write_test_material_rows,
                    freeze_frame_material_table,
                    ensure_material_table_buffer_handle,
                )
                    .chain(),
            );

        app.update();
        let first_handle = app
            .world()
            .resource::<MaterialTableBuffer>()
            .handle
            .clone()
            .expect("the first frame should create the active material table buffer");
        let first_capacity = app.world().resource::<MaterialTableBuffer>().capacity;
        app.world_mut().resource_mut::<TestMaterialRows>().0 = first_capacity;

        app.update();
        let table_buffer = app.world().resource::<MaterialTableBuffer>();
        assert_eq!(table_buffer.handle.as_ref(), Some(&first_handle));
        let pending_handle = table_buffer
            .pending
            .as_ref()
            .map(|pending| pending.handle.clone())
            .expect("the full-capacity frame should prepare a larger buffer");
        assert_ne!(pending_handle, first_handle);

        app.update();
        let table_buffer = app.world().resource::<MaterialTableBuffer>();
        assert_eq!(table_buffer.handle.as_ref(), Some(&pending_handle));
        assert!(table_buffer.pending.is_none());
        assert!(table_buffer.capacity > first_capacity);
    }

    #[test]
    fn growth_prepares_a_pending_handle_without_rebinding_the_active_buffer() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Assets::<ShaderBuffer>::default())
            .init_resource::<FrameMaterialTableBuild>()
            .init_resource::<MaterialTableBuffer>()
            .insert_resource(TestMaterialRows(1))
            .add_systems(
                Update,
                (
                    activate_prepared_material_table_buffer,
                    clear_frame_material_table,
                    write_test_material_rows,
                    freeze_frame_material_table,
                    ensure_material_table_buffer_handle,
                )
                    .chain(),
            );

        app.update();
        let active_handle = app.world().resource::<MaterialTableBuffer>().handle.clone();
        assert!(active_handle.is_some());
        let active_handle = active_handle.unwrap_or_default();
        let active_capacity = app.world().resource::<MaterialTableBuffer>().capacity;
        app.world_mut().resource_mut::<TestMaterialRows>().0 = active_capacity.saturating_add(1);

        app.update();

        let table_buffer = app.world().resource::<MaterialTableBuffer>();
        assert_eq!(table_buffer.handle.as_ref(), Some(&active_handle));
        assert!(table_buffer.pending.is_some());
    }

    #[test]
    fn mixed_producers_share_one_row_order() {
        let mut builder = FrameMaterialTableBuilder::default();
        builder.clear(INVALID_GPU_MATERIAL_SLOT - 1);
        let inputs = [
            TestInput {
                key:      ProducerKey::Sdf(0),
                material: material_with_values(0),
            },
            TestInput {
                key:      ProducerKey::Text(0),
                material: material_with_values(1),
            },
            TestInput {
                key:      ProducerKey::Shape(0),
                material: material_with_values(2),
            },
        ];

        let appended: Vec<_> = inputs
            .iter()
            .map(|input| append_material_slot(&mut builder, input))
            .collect();

        assert!(matches!(
            appended[0],
            MaterialSlotAppend::Appended(MaterialSlotAppended {
                key: ProducerKey::Sdf(0),
                slot: MaterialSlotId(0),
                ..
            })
        ));
        assert!(matches!(
            appended[1],
            MaterialSlotAppend::Appended(MaterialSlotAppended {
                key: ProducerKey::Text(0),
                slot: MaterialSlotId(1),
                ..
            })
        ));
        assert!(matches!(
            appended[2],
            MaterialSlotAppend::Appended(MaterialSlotAppended {
                key: ProducerKey::Shape(0),
                slot: MaterialSlotId(2),
                ..
            })
        ));
        assert_eq!(builder.freeze().row_count(), inputs.len());
    }

    #[test]
    fn hidden_clipped_missing_or_removed_sources_append_no_rows() {
        let mut builder = FrameMaterialTableBuilder::default();
        builder.clear(INVALID_GPU_MATERIAL_SLOT - 1);
        let live_sources = [Some(material_with_values(0)), None, None, None];
        for material in live_sources.iter().flatten() {
            let _ = builder.append_values(MaterialSlotValues::from(material));
        }
        assert_eq!(builder.freeze().row_count(), 1);
    }

    #[test]
    fn projection_matches_bevy_standard_material_uniform_values() {
        let material = material_with_values(7);
        let images = RenderAssets::<GpuImage>::default();
        let uniform: StandardMaterialUniform = material.as_bind_group_shader_type(&images);
        let values = MaterialSlotValues::from(&material);

        assert_eq!(
            values,
            MaterialSlotValues::from_standard_material_uniform(uniform.clone())
        );
        assert_eq!(values.base_color, uniform.base_color);
        assert_eq!(values.emissive, uniform.emissive);
        assert_eq!(values.attenuation_color, uniform.attenuation_color);
        assert_eq!(values.uv_transform, uniform.uv_transform);
        assert_eq!(values.reflectance, uniform.reflectance);
        assert_same_f32(values.roughness, uniform.roughness);
        assert_same_f32(values.metallic, uniform.metallic);
        assert_same_f32(values.diffuse_transmission, uniform.diffuse_transmission);
        assert_same_f32(values.specular_transmission, uniform.specular_transmission);
        assert_same_f32(values.thickness, uniform.thickness);
        assert_same_f32(values.ior, uniform.ior);
        assert_same_f32(values.attenuation_distance, uniform.attenuation_distance);
        assert_same_f32(values.clearcoat, uniform.clearcoat);
        assert_same_f32(
            values.clearcoat_perceptual_roughness,
            uniform.clearcoat_perceptual_roughness,
        );
        assert_same_f32(values.anisotropy_strength, uniform.anisotropy_strength);
        assert_eq!(values.anisotropy_rotation, uniform.anisotropy_rotation);
    }

    #[test]
    fn scalar_value_changes_do_not_change_compatibility() {
        let first = MaterialSlotCandidate::from(&material_with_values(1));
        let second = MaterialSlotCandidate::from(&material_with_values(2));

        assert_ne!(first.values, second.values);
        assert_eq!(first.pipeline_compatibility, second.pipeline_compatibility);
        assert_eq!(first.resource_compatibility, second.resource_compatibility);
    }

    #[test]
    fn alpha_culling_and_textures_change_compatibility_not_table_values() {
        let base = material_with_values(0);
        let mut changed = base.clone();
        changed.alpha_mode = AlphaMode::Mask(0.33);
        changed.double_sided = true;
        changed.cull_mode = Some(Face::Front);
        changed.base_color_texture = Some(Handle::default());

        assert_ne!(
            PipelineCompatibility::from(&base),
            PipelineCompatibility::from(&changed)
        );
        assert_ne!(
            ResourceCompatibility::from(&base),
            ResourceCompatibility::from(&changed)
        );
        assert_eq!(
            MaterialSlotValues::from(&base),
            MaterialSlotValues::from(&changed)
        );
    }

    #[test]
    fn all_alpha_modes_and_cull_modes_are_preserved() {
        let alpha_modes = [
            AlphaMode::Opaque,
            AlphaMode::Mask(0.25),
            AlphaMode::Blend,
            AlphaMode::Premultiplied,
            AlphaMode::AlphaToCoverage,
            AlphaMode::Add,
            AlphaMode::Multiply,
        ];
        for alpha_mode in alpha_modes {
            let material = StandardMaterial {
                alpha_mode,
                ..default()
            };
            assert_eq!(
                PipelineCompatibility::from(&material).alpha,
                BatchAlphaMode::from(alpha_mode)
            );
        }

        for cull_mode in [None, Some(Face::Back), Some(Face::Front)] {
            let material = StandardMaterial {
                cull_mode,
                ..default()
            };
            assert_eq!(PipelineCompatibility::from(&material).cull_mode, cull_mode);
        }
    }

    #[test]
    fn row_limit_drop_is_explicit_and_counted() {
        let mut builder = FrameMaterialTableBuilder::default();
        builder.clear(1);
        let values = MaterialSlotValues::from(&StandardMaterial::default());

        assert!(matches!(
            builder.append_values(values),
            FrameMaterialSlotAppend::Appended(MaterialSlotId(0))
        ));
        assert_eq!(
            builder.append_values(values),
            FrameMaterialSlotAppend::DroppedLimit
        );
        assert_eq!(builder.dropped_record_count(), 1);
        assert_eq!(builder.freeze().row_count(), 1);
    }

    #[derive(Resource)]
    struct ProbeKeys {
        offset: u32,
        count:  u32,
    }

    fn probe_write_rows(keys: Res<ProbeKeys>, mut build: ResMut<FrameMaterialTableBuild>) {
        let values = MaterialSlotValues::from(&StandardMaterial::default());
        for index in 0..keys.count {
            let _ = build
                .builder_mut()
                .upsert_values(MaterialSourceKey::Test(keys.offset + index), values);
        }
    }

    #[derive(Resource)]
    struct PanelRespawnKeys {
        panel: Entity,
        count: u32,
    }

    fn write_panel_respawn_rows(
        keys: Res<PanelRespawnKeys>,
        mut build: ResMut<FrameMaterialTableBuild>,
    ) {
        let values = MaterialSlotValues::from(&StandardMaterial::default());
        for element_index in 0..keys.count.to_usize() {
            let source = SdfMaterialSourceKey {
                panel:           keys.panel,
                element:         ElementIndex::from(element_index),
                divider_ordinal: None,
                role:            SdfMaterialRole::Fill,
            };
            let _ = build.builder_mut().upsert_values(source.into(), values);
        }
    }

    #[test]
    fn probe_rekey_drop() {
        let mut app = material_table_test_app();
        app.insert_resource(ProbeKeys {
            offset: 0,
            count:  PROBE_SOURCE_COUNT,
        })
        .add_systems(
            Update,
            (
                clear_frame_material_table,
                probe_write_rows,
                freeze_frame_material_table,
                ensure_material_table_buffer_handle,
            )
                .chain(),
        );

        app.update();
        assert_material_table_frame(&app, PROBE_SOURCE_COUNT);
        app.update();
        assert_material_table_frame(&app, PROBE_SOURCE_COUNT);
        // Changing `ProbeKeys::offset` replaces every positional source identity.
        app.world_mut().resource_mut::<ProbeKeys>().offset = 10_000;
        app.update();
        assert_material_table_rekey_frame(&app, PROBE_SOURCE_COUNT);
        app.update();
        assert_material_table_frame(&app, PROBE_SOURCE_COUNT);
        app.update();
        assert_material_table_frame(&app, PROBE_SOURCE_COUNT);
        app.world_mut().resource_mut::<ProbeKeys>().offset = 20_000;
        app.update();
        assert_material_table_rekey_frame(&app, PROBE_SOURCE_COUNT);
        app.update();
        assert_material_table_frame(&app, PROBE_SOURCE_COUNT);
    }

    #[test]
    fn cold_start_larger_than_default_capacity_never_drops_material_rows() {
        let mut app = material_table_test_app();
        app.insert_resource(ProbeKeys {
            offset: 0,
            count:  PROBE_SOURCE_COUNT,
        })
        .add_systems(
            Update,
            (
                clear_frame_material_table,
                probe_write_rows,
                freeze_frame_material_table,
                ensure_material_table_buffer_handle,
            )
                .chain(),
        );

        let source_count = app.world().resource::<ProbeKeys>().count;
        assert!(source_count > DEFAULT_TABLE_CAPACITY);

        app.update();

        assert_material_table_frame(&app, PROBE_SOURCE_COUNT);
    }

    #[test]
    fn panel_respawn_rekeys_sdf_material_rows_without_drops() {
        let mut app = material_table_test_app();
        let first_panel = app.world_mut().spawn_empty().id();
        app.insert_resource(PanelRespawnKeys {
            panel: first_panel,
            count: PROBE_SOURCE_COUNT,
        })
        .add_systems(
            Update,
            (
                clear_frame_material_table,
                write_panel_respawn_rows,
                freeze_frame_material_table,
                ensure_material_table_buffer_handle,
            )
                .chain(),
        );

        app.update();
        assert_material_table_frame(&app, PROBE_SOURCE_COUNT);
        app.update();
        assert_material_table_frame(&app, PROBE_SOURCE_COUNT);

        assert!(app.world_mut().despawn(first_panel));
        let second_panel = app.world_mut().spawn_empty().id();
        assert_ne!(second_panel, first_panel);
        app.world_mut().resource_mut::<PanelRespawnKeys>().panel = second_panel;

        app.update();
        assert_material_table_rekey_frame(&app, PROBE_SOURCE_COUNT);
    }

    #[test]
    fn configured_row_limit_is_applied_by_frame_build_clear() {
        let mut app = App::new();
        app.init_resource::<FrameMaterialTableBuild>();
        let row_size = u64::try_from(MaterialSlotValues::shader_size_bytes()).unwrap_or(u64::MAX);
        set_material_table_row_limit_from_storage_buffer_bytes(&mut app, row_size);
        let values = MaterialSlotValues::from(&StandardMaterial::default());
        let mut build = app.world_mut().resource_mut::<FrameMaterialTableBuild>();

        build.clear();
        assert!(matches!(
            build.builder_mut().append_values(values),
            FrameMaterialSlotAppend::Appended(MaterialSlotId(0))
        ));
        assert_eq!(
            build.builder_mut().append_values(values),
            FrameMaterialSlotAppend::DroppedLimit
        );
        build.freeze();
        assert_eq!(build.dropped_record_count(), 1);
    }

    #[test]
    fn append_after_freeze_panics() {
        let mut builder = FrameMaterialTableBuilder::default();
        builder.clear(INVALID_GPU_MATERIAL_SLOT - 1);
        let _ = builder.freeze();
        let panic_result = std::panic::catch_unwind(move || {
            let _ = builder.append_values(MaterialSlotValues::default());
        });
        assert!(panic_result.is_err());
    }

    #[test]
    fn frame_atomic_extract_clones_rows_with_current_handle() {
        let mut build = FrameMaterialTableBuild::default();
        build.clear();
        let _ = build
            .builder_mut()
            .append_values(MaterialSlotValues::from(&StandardMaterial::default()));
        build.freeze();
        let table = build.table().clone();
        let handle = Handle::<ShaderBuffer>::default();
        let extracted = ExtractedFrameMaterialTable {
            table:  table.clone(),
            handle: Some(handle),
            data:   Vec::new(),
        };

        assert_eq!(extracted.table.row_count(), 1);
        assert_eq!(extracted.handle, Some(Handle::<ShaderBuffer>::default()));
        assert_eq!(table.row_count(), extracted.table.row_count());
    }

    #[test]
    fn registry_purges_dead_batch_entities() {
        let mut world = World::new();
        let mut registry = BatchMaterialTableRegistry::default();
        let live = world.spawn_empty().id();
        let dead = world.spawn_empty().id();
        let _ = world.despawn(dead);
        registry.register_path(live, Handle::default());
        registry.register_path(dead, Handle::default());

        registry.purge_dead_with(|entity| world.get_entity(entity).is_ok());

        assert_eq!(registry.len(), 1);
        registry.unregister_path(live);
        assert!(registry.is_empty());
    }

    #[test]
    fn registry_does_not_accumulate_stale_entries_across_frames() {
        let mut world = World::new();
        let mut registry = BatchMaterialTableRegistry::default();
        for _ in 0..10 {
            let entity = world.spawn_empty().id();
            registry.register_path(entity, Handle::default());
            let _ = world.despawn(entity);
            registry.purge_dead_with(|entity| world.get_entity(entity).is_ok());
            assert!(registry.is_empty());
        }
    }

    #[test]
    fn rebind_updates_registered_material_once_per_table_handle() {
        let mut materials = Assets::<PathExtendedMaterial>::default();
        let material = materials.add(test_path_material());
        let handle = Handle::<ShaderBuffer>::default();
        let mut world = World::new();
        let entity = world.spawn_empty().id();
        let mut registry = BatchMaterialTableRegistry::default();
        registry.register_path(entity, material);

        let first_rebind = rebind_registered_path_materials(&mut registry, &handle, &mut materials);
        let second_rebind =
            rebind_registered_path_materials(&mut registry, &handle, &mut materials);

        assert_eq!(first_rebind, 1);
        assert_eq!(second_rebind, 0);
        assert!(!registry.pending_rebind);
        let registered = registry
            .path_materials
            .get(&entity)
            .expect("registered material should remain tracked");
        assert_eq!(registered.bound_handle.as_ref(), Some(&handle));
    }

    #[test]
    fn retained_batch_asset_allocation_finishes_before_asset_events() {
        let mut app = App::new();
        app.add_plugins(AssetPlugin::default());
        app.add_plugins(VisibilityPlugin);
        app.add_plugins((
            MaterialTablePlugin,
            TextRenderPlugin,
            PanelShapePlugin,
            FillBatchPlugin,
            ImageBatchPlugin,
        ));
        let snapshot = with_initialized_post_update(&mut app, inspect_batch_schedule);

        for required in [
            "register_path_batch_materials",
            "register_sdf_batch_materials",
            "reconcile_panel_line_batches",
            "reconcile_sdf_batch_entities",
            "reconcile_image_batch_entities",
            "commit_batch_buffers",
            "commit_panel_line_batch_buffers",
            "commit_sdf_batch_buffers",
            "commit_image_batch_buffers",
        ] {
            assert!(
                snapshot
                    .resources
                    .iter()
                    .any(|name| name.contains(required)),
                "{required} should be inside BatchResourcesReady"
            );
        }

        for required in [
            "update_panel_text_batches",
            "reconcile_panel_line_batches",
            "reconcile_sdf_batch_entities",
            "reconcile_image_batch_entities",
        ] {
            assert!(
                snapshot
                    .asset_writes
                    .iter()
                    .any(|name| name.contains(required)),
                "{required} should be inside BatchAssetWrites"
            );
        }
        assert!(
            snapshot
                .append_ready
                .iter()
                .any(|name| name.contains("activate_prepared_material_table_buffer")),
            "activate_prepared_material_table_buffer must promote a pending ShaderBuffer before producers append"
        );
    }

    #[test]
    fn retained_gpu_writes_run_between_buffer_and_material_preparation() {
        let mut app = App::new();
        app.add_plugins(AssetPlugin::default());
        app.insert_sub_app(RenderApp, SubApp::new());
        app.add_plugins((
            MaterialTablePlugin,
            MaterialPlugin::<PathExtendedMaterial>::default(),
            FillBatchPlugin,
            ImageBatchPlugin,
        ));
        app.finish();

        let snapshot = with_initialized_render(&mut app, inspect_direct_upload_schedule);

        assert!(
            snapshot.all_in_set,
            "every direct upload system should be inside DirectGpuBufferUploads: {:?}",
            snapshot.systems,
        );
        assert!(
            snapshot.systems.len() == 2,
            "both direct upload systems should exist: {:?}",
            snapshot.systems,
        );
        assert!(
            snapshot.after_buffer_preparation,
            "Bevy GPU buffer preparation must precede DirectGpuBufferUploads"
        );
        assert!(
            snapshot.before_material_preparation,
            "Bevy retained material preparation must follow DirectGpuBufferUploads"
        );
    }

    #[test]
    fn binding_numbers_are_unique_and_match_wgsl_mirror() {
        let bindings = [
            PATH_UNIFORM_BINDING,
            PATH_CURVES_BINDING,
            PATH_BANDS_BINDING,
            PATH_RECORDS_BINDING,
            PATH_INSTANCES_BINDING,
            PATH_RUN_RECORDS_BINDING,
            MATERIAL_TABLE_BINDING,
            SDF_RENDER_RECORDS_BINDING,
            SDF_MESH_BINDING,
        ];
        for (index, binding) in bindings.iter().enumerate() {
            assert!(!bindings[index + 1..].contains(binding));
        }

        let wgsl = include_str!("material_table.wgsl");
        assert!(wgsl.contains("const MATERIAL_TABLE_BINDING: u32 = 106u;"));
        assert!(wgsl.contains("const INVALID_GPU_MATERIAL_SLOT: u32 = 4294967295u;"));
    }

    #[test]
    fn material_slot_values_and_standard_material_uniform_have_stable_shader_sizes() {
        assert_eq!(
            MaterialSlotValues::min_size().get(),
            MATERIAL_SLOT_VALUES_SHADER_SIZE_BYTES
        );
        assert_eq!(
            StandardMaterialUniform::min_size().get(),
            STANDARD_MATERIAL_UNIFORM_SHADER_SIZE_BYTES
        );
        let wgsl = include_str!("material_table.wgsl");
        assert_eq!(
            wgsl_material_slot_fields(wgsl),
            MATERIAL_SLOT_VALUES_WGSL_FIELDS
        );
    }

    #[test]
    fn measurement_harness_emits_structured_rows_without_thresholds() {
        let measurements = emit_material_table_measurements();
        assert_eq!(measurements.len(), 3);
        assert!(measurements.iter().all(|measurement| measurement.rows > 0));
        assert!(
            measurements
                .iter()
                .all(|measurement| measurement.upload_bytes >= measurement.rows)
        );
    }
}
