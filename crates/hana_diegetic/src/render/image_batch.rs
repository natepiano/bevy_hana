//! CPU image batch records and storage-buffer growth helpers.
//!
//! Image batches always render through `AlphaMode::Blend`: authored images have
//! no alpha-mode source, so `ImageBatchKey` omits alpha mode and orders records
//! with per-record `DrawCommandDepth::oit_depth_offset()`. SDF and text batches
//! have family-specific alpha remaps for prepass/shadow pipeline layouts; image
//! batches need no remap because every image draw is already transparent.

use std::collections::HashSet;

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::visibility::VisibilitySystems;
use bevy::image::Image;
use bevy::light::NotShadowCaster;
use bevy::mesh::Indices;
use bevy::pbr::MaterialPlugin;
use bevy::picking::Pickable;
use bevy::prelude::App;
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
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Vec4;
use bevy::prelude::Vec4Swizzles;
use bevy::prelude::Visibility;
use bevy::render::render_resource::PrimitiveTopology;
use bevy::render::render_resource::ShaderSize;
use bevy::render::render_resource::ShaderType;
use bevy::render::storage::ShaderBuffer;
use bevy::transform::TransformSystems;
use hana_kana::ToU32;
use hana_kana::ToUsize;

use super::BatchRenderLayers;
use super::CommandIndex;
use super::Dirty;
use super::VisualShadow;
use super::batch_store;
use super::batch_store::Batch;
use super::batch_store::BatchEntry;
use super::batch_store::BatchStore;
use super::batch_store::MemberBatch;
use super::batch_store::MemberFamily;
use super::batch_store::MemberRecord;
use super::clip;
use super::constants::TEXT_Z_OFFSET;
use super::draw_order::DrawCommandDepth;
use super::draw_order::DrawOrderIndex;
use super::draw_order::DrawZIndexRank;
use super::image_material;
use super::image_material::ImageExtendedMaterial;
use super::material_table;
use super::material_table::BatchAssetWrites;
use super::material_table::BatchResourcesReady;
use super::material_table::RetainedBatchBufferUploads;
use super::precompose;
use crate::cascade::Resolved;
use crate::layout::BoundingBox;
use crate::layout::DrawZIndex;
use crate::layout::RenderCommand;
use crate::layout::RenderCommandKind;
use crate::layout::ShadowCasting;
use crate::panel::BatchSummary;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::DiegeticPanel;
use crate::panel::DiegeticPerfStats;
use crate::panel::PanelLayout;
use crate::panel::PanelPrecomposeCache;
use crate::widgets::VisualOverrideIndex;
use crate::widgets::VisualSlotOverride;

/// Per-command image record identity.
///
/// `ImageBatchStore` uses this key as the membership index for moving records
/// between `ImageBatchKey` buckets.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct ImageRecordKey {
    /// Panel entity whose command stream produced this image record.
    pub panel:         Entity,
    /// Command index of the image or precompose command.
    pub command_index: CommandIndex,
}

/// UV rectangle sampled by `ImageRenderRecord`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ImageUvRect {
    /// Minimum UV corner.
    pub min: Vec2,
    /// Maximum UV corner.
    pub max: Vec2,
}

impl Default for ImageUvRect {
    fn default() -> Self {
        Self {
            min: Vec2::ZERO,
            max: Vec2::ONE,
        }
    }
}

impl ImageUvRect {
    const fn as_vec4(self) -> Vec4 { Vec4::new(self.min.x, self.min.y, self.max.x, self.max.y) }
}

/// Key for one image batch.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ImageBatchKey {
    /// Texture sampled by every record in this image batch.
    pub texture:      Handle<Image>,
    /// Render layers copied from the owning panel.
    pub layers:       BatchRenderLayers,
    /// Shadow participation for this batch.
    pub shadow:       VisualShadow,
    /// Authored z-index splitter for this batch's image records.
    pub z_index:      DrawZIndex,
    /// Dense panel-local rank for `z_index`, used by the batch material
    /// `StandardMaterial::depth_bias`.
    pub z_index_rank: DrawZIndexRank,
}

impl ImageBatchKey {
    /// `StandardMaterial::depth_bias` value for the image batch material.
    #[must_use]
    pub(super) fn depth_bias(&self) -> f32 { self.z_index_rank.screen_depth_bias().get() }
}

/// Marker inserted on private image batch entities.
#[derive(Component)]
struct DiegeticImageBatch;

/// CPU-side image record retained by `ImageBatchStore`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ResolvedImageRecord {
    /// Per-command image record identity.
    pub record_key:      ImageRecordKey,
    /// Panel-local transform after applying scale, anchor, Y flip, and Z offset.
    pub local_transform: Transform,
    /// World-unit quad size.
    pub size:            Vec2,
    /// Linear RGBA tint multiplied after texture sRGB decode.
    pub tint:            Vec4,
    /// Source UV rectangle, defaulting to the full texture.
    pub uv_rect:         ImageUvRect,
    /// `DrawCommandDepth` for this command.
    pub draw_depth:      DrawCommandDepth,
    /// World transform written after `TransformSystems::Propagate`.
    pub transform:       Mat4,
}

impl ResolvedImageRecord {
    /// Builds a CPU-retained image record with an identity world transform.
    #[must_use]
    const fn new(
        record_key: ImageRecordKey,
        local_transform: Transform,
        size: Vec2,
        tint: Vec4,
        uv_rect: ImageUvRect,
        draw_depth: DrawCommandDepth,
    ) -> Self {
        Self {
            record_key,
            local_transform,
            size,
            tint,
            uv_rect,
            draw_depth,
            transform: Mat4::IDENTITY,
        }
    }

    fn update_world_transform(&mut self, panel_transform: &GlobalTransform) {
        self.transform = panel_transform.to_matrix() * self.local_transform.to_matrix();
    }
}

impl MemberRecord for ResolvedImageRecord {
    fn panel(&self) -> Entity { self.record_key.panel }

    fn transform(&self) -> Mat4 { self.transform }

    fn update_world_transform(&mut self, panel_transform: &GlobalTransform) {
        Self::update_world_transform(self, panel_transform);
    }
}

/// GPU mirror for one batched image quad.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
struct ImageRenderRecord {
    /// World-space transform used by the vertex shader.
    transform:        Mat4,
    /// Quad size in local record space.
    size:             Vec2,
    /// Source UV rectangle `[u_min, v_min, u_max, v_max]`.
    uv_rect:          Vec4,
    /// Linear RGBA tint multiplied after texture sRGB decode.
    tint:             Vec4,
    /// Clip-space depth nudge in layer units for non-OIT views.
    clip_depth_nudge: f32,
    /// Per-record OIT position-z offset for coplanar ordering.
    oit_depth_offset: f32,
}

impl ImageRenderRecord {
    /// Converts the CPU retained record to the GPU storage-buffer mirror.
    #[must_use]
    fn from_resolved(record: &ResolvedImageRecord, first_draw_order_index: DrawOrderIndex) -> Self {
        Self {
            transform:        image_record_transform(record),
            size:             record.size,
            uv_rect:          record.uv_rect.as_vec4(),
            tint:             record.tint,
            clip_depth_nudge: record.draw_depth.clip_depth_nudge().get()
                - first_draw_order_index.clip_depth_nudge().get(),
            oit_depth_offset: record.draw_depth.oit_depth_offset().get(),
        }
    }

    /// Capacity-tail record that rasterizes no pixels.
    #[must_use]
    const fn padded() -> Self {
        Self {
            transform:        Mat4::ZERO,
            size:             Vec2::ZERO,
            uv_rect:          Vec4::ZERO,
            tint:             Vec4::ZERO,
            clip_depth_nudge: 0.0,
            oit_depth_offset: 0.0,
        }
    }
}

const _: () = assert!(ImageRenderRecord::SHADER_SIZE.get() == 128);

/// GPU assets owned by one image batch.
#[derive(Debug)]
pub(super) struct ImageBatchResources {
    /// `ImageRenderRecord` storage buffer.
    pub records:  Handle<ShaderBuffer>,
    /// Degenerate mesh sized to the current record capacity.
    pub mesh:     Handle<Mesh>,
    /// `ImageExtendedMaterial` bound to `records`.
    pub material: Handle<ImageExtendedMaterial>,
    /// Record capacity represented by `records`.
    pub capacity: u32,
}

/// CPU record set and GPU handles for one `ImageBatchKey`.
#[derive(Debug, Default)]
pub(super) struct ImageBatch {
    /// Batch render entity; `None` before the first GPU allocation.
    pub entity:             Option<Entity>,
    /// GPU handles; `None` before the first GPU allocation.
    pub gpu:                Option<ImageBatchResources>,
    /// Record-buffer upload state for this batch.
    pub record_upload:      Dirty,
    /// Bounds recomputation state for this batch.
    pub bounds_update:      Dirty,
    /// Lowest `DrawOrderIndex` in this batch.
    ///
    /// `ImageRenderRecord::clip_depth_nudge` is uploaded relative to this value;
    /// `ImageRenderRecord::oit_depth_offset` stays panel-absolute.
    first_draw_order_index: DrawOrderIndex,
    records:                Vec<ResolvedImageRecord>,
}

impl ImageBatch {
    /// CPU records in upload order.
    #[must_use]
    pub(super) fn records(&self) -> &[ResolvedImageRecord] { &self.records }

    /// Number of live image records in this batch.
    #[must_use]
    fn record_count(&self) -> u32 { self.records.len().to_u32() }

    /// Whether this batch has no live image records.
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

    fn position_of(&self, key: ImageRecordKey) -> Option<usize> {
        self.records
            .iter()
            .position(|record| record.record_key == key)
    }

    fn sort_records(&mut self) {
        self.records.sort_by(|left, right| {
            left.draw_depth
                .draw_order_index()
                .cmp(&right.draw_depth.draw_order_index())
                .then(
                    left.record_key
                        .command_index
                        .cmp(&right.record_key.command_index),
                )
        });
    }

    fn upsert_record(&mut self, mut record: ResolvedImageRecord) {
        if let Some(position) = self.position_of(record.record_key) {
            record.transform = self.records[position].transform;
            if self.records[position] == record {
                return;
            }
            self.records[position] = record;
        } else {
            self.records.push(record);
        }
        self.sort_records();
        self.refresh_first_draw_order_index();
        self.record_upload.mark();
        self.bounds_update.mark();
    }

    fn remove_record(&mut self, key: ImageRecordKey) {
        if let Some(position) = self.position_of(key) {
            self.records.remove(position);
            self.refresh_first_draw_order_index();
            self.record_upload.mark();
            self.bounds_update.mark();
        }
    }

    /// World-space union from each image record's transformed quad corners.
    #[must_use]
    fn world_bounds(&self) -> Option<(Vec3, Vec3)> {
        if self.records.is_empty() {
            return None;
        }
        let mut min = Vec3::MAX;
        let mut max = Vec3::MIN;
        for record in &self.records {
            let transform = image_record_transform(record);
            for corner in centered_corners(record.size) {
                let world = transform * Vec4::new(corner.x, corner.y, 0.0, 1.0);
                min = min.min(world.xyz());
                max = max.max(world.xyz());
            }
        }
        Some((min, max))
    }
}

impl BatchEntry for ImageBatch {
    fn is_empty(&self) -> bool { Self::is_empty(self) }

    fn entity(&self) -> Option<Entity> { self.entity }
}

impl Batch for ImageBatch {
    type MemberKey = ImageRecordKey;
    type Payload = ResolvedImageRecord;

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

impl MemberBatch for ImageBatch {
    type Record = ResolvedImageRecord;

    fn records_mut(&mut self) -> &mut [Self::Record] { &mut self.records }

    fn record_upload_mut(&mut self) -> &mut Dirty { &mut self.record_upload }

    fn bounds_update(&self) -> Dirty { self.bounds_update }

    fn bounds_update_mut(&mut self) -> &mut Dirty { &mut self.bounds_update }

    fn world_bounds(&self) -> Option<(Vec3, Vec3)> { Self::world_bounds(self) }
}

/// Store that maps image records to `ImageBatchKey` batches.
#[derive(Debug, Default, Resource)]
pub(super) struct ImageBatchStore(BatchStore<ImageBatchKey, ImageBatch>);

impl ImageBatchStore {
    /// Inserts or moves one retained image record.
    fn upsert_record(&mut self, batch_key: ImageBatchKey, record: ResolvedImageRecord) {
        let record_key = record.record_key;
        self.0.upsert(batch_key, record_key, record);
    }

    /// Removes every retained image record absent from the current append pass.
    fn retain_records(&mut self, active: &HashSet<ImageRecordKey>) { self.0.retain(active); }

    /// All image batches.
    #[cfg(test)]
    pub(super) fn batches(&self) -> impl Iterator<Item = (&ImageBatchKey, &ImageBatch)> {
        self.0.batches()
    }

    /// All image batches, mutable.
    fn batches_mut(&mut self) -> impl Iterator<Item = (&ImageBatchKey, &mut ImageBatch)> {
        self.0.batches_mut()
    }

    /// One image batch by key, mutable.
    #[cfg(test)]
    fn get_mut(&mut self, key: &ImageBatchKey) -> Option<&mut ImageBatch> { self.0.get_mut(key) }

    /// Drops empty batch entries, returning their entities for despawn.
    fn take_empty_batches(&mut self) -> Vec<Entity> { self.0.take_empty_batches() }
}

struct ImageMemberFamily;

impl MemberFamily for ImageMemberFamily {
    type Key = ImageBatchKey;
    type Batch = ImageBatch;
    type Store = ImageBatchStore;
    type Marker = DiegeticImageBatch;

    fn store_mut(store: &mut Self::Store) -> &mut BatchStore<Self::Key, Self::Batch> {
        &mut store.0
    }
}

/// Plugin that routes computed image commands into [`ImageBatchStore`].
pub(super) struct ImageBatchPlugin;

impl Plugin for ImageBatchPlugin {
    fn build(&self, app: &mut App) {
        // Never `init_asset` bevy-owned asset types here: `init_asset` replaces
        // any existing `Assets` store, orphaning every handle created by
        // plugins that built earlier. The host app's render setup owns
        // `Assets<Mesh>` and `Assets<ShaderBuffer>`; headless tests init them
        // explicitly.
        app.init_resource::<ImageBatchStore>()
            .init_resource::<DiegeticPerfStats>()
            .init_resource::<VisualOverrideIndex>()
            .add_plugins(MaterialPlugin::<ImageExtendedMaterial>::default())
            .add_systems(
                PostUpdate,
                // Ordered after the precompose cache systems (whose
                // `PanelPrecomposeCache` this reads), not the whole
                // `PanelChildSystems::Build` set: the panel-shape batch systems
                // are members of both `Build` and `BatchResourcesReady`, so an
                // `after(Build).before(BatchResourcesReady)` edge is unsolvable.
                route_image_batch_records
                    .after(precompose::cleanup_retired_precompose_images)
                    .before(TransformSystems::Propagate)
                    .before(BatchResourcesReady),
            )
            .add_systems(
                PostUpdate,
                batch_store::update_batch_world_transforms::<ImageMemberFamily>
                    .after(TransformSystems::Propagate)
                    .in_set(BatchResourcesReady),
            )
            .add_systems(
                PostUpdate,
                reconcile_image_batch_entities
                    .after(batch_store::update_batch_world_transforms::<ImageMemberFamily>)
                    .in_set(BatchAssetWrites)
                    .before(VisibilitySystems::CalculateBounds)
                    .in_set(BatchResourcesReady),
            )
            .add_systems(
                PostUpdate,
                batch_store::update_batch_bounds::<ImageMemberFamily>
                    .after(reconcile_image_batch_entities)
                    .after(VisibilitySystems::CalculateBounds)
                    .before(VisibilitySystems::CheckVisibility)
                    .in_set(BatchResourcesReady),
            )
            .add_systems(
                PostUpdate,
                commit_image_batch_buffers
                    .after(reconcile_image_batch_entities)
                    .before(batch_store::update_batch_bounds::<ImageMemberFamily>)
                    .in_set(BatchResourcesReady),
            );
    }
}

fn route_image_batch_records(
    panels: Query<(
        Entity,
        &DiegeticPanel,
        &ComputedDiegeticPanel,
        &PanelPrecomposeCache,
        Option<&RenderLayers>,
        Option<&Visibility>,
        Option<&Resolved<ShadowCasting>>,
    )>,
    visual_overrides: Res<VisualOverrideIndex>,
    mut store: ResMut<ImageBatchStore>,
) {
    let mut active_records = HashSet::new();
    for (
        panel_entity,
        panel,
        computed,
        precompose_cache,
        panel_layers,
        panel_visibility,
        panel_shadow_casting,
    ) in &panels
    {
        let PanelLayout::Solved(result) = computed.layout() else {
            continue;
        };
        if matches!(panel_visibility, Some(Visibility::Hidden)) {
            continue;
        }
        let layers = panel_layers.cloned().unwrap_or(RenderLayers::layer(0));
        let shadow_casting = panel_shadow_casting.map_or(ShadowCasting::On, |resolved| resolved.0);
        for (batch_key, record) in collect_panel_image_records(
            panel_entity,
            panel,
            result.commands.as_slice(),
            computed,
            precompose_cache,
            layers.clone(),
            shadow_casting,
            &visual_overrides,
        ) {
            active_records.insert(record.record_key);
            store.upsert_record(batch_key, record);
        }
    }
    store.retain_records(&active_records);
}

fn collect_panel_image_records(
    panel_entity: Entity,
    panel: &DiegeticPanel,
    commands: &[RenderCommand],
    computed: &ComputedDiegeticPanel,
    precompose_cache: &PanelPrecomposeCache,
    layers: RenderLayers,
    shadow_casting: ShadowCasting,
    visual_overrides: &VisualOverrideIndex,
) -> Vec<(ImageBatchKey, ResolvedImageRecord)> {
    let clip_rects = clip::compute_clip_rects(commands);
    let viewport = clip::panel_viewport(panel);
    let points_to_world = panel.points_to_world();
    let (anchor_x, anchor_y) = panel.anchor_offsets();
    commands
        .iter()
        .enumerate()
        .filter_map(|(command_index, command)| {
            clip::effective_clip(command.bounds, clip_rects[command_index], viewport)?;
            let draw_depth = computed.draw_order().depth_for(command_index)?;
            let (texture, tint) = image_record_source(command, precompose_cache)?;
            let slot_override = visual_overrides.get(panel_entity, command.element_idx);
            let (texture, tint, local_transform) = apply_image_visual_override(
                slot_override,
                texture,
                tint,
                local_transform_from_bounds(command.bounds, points_to_world, anchor_x, anchor_y),
            );
            let batch_key = ImageBatchKey {
                texture,
                layers: BatchRenderLayers(layers.clone()),
                shadow: shadow_casting.into(),
                z_index: draw_depth.z_index(),
                z_index_rank: draw_depth.z_index_rank(),
            };
            let record_key = ImageRecordKey {
                panel:         panel_entity,
                command_index: CommandIndex::from(command_index),
            };
            Some((
                batch_key,
                ResolvedImageRecord::new(
                    record_key,
                    local_transform,
                    image_size_from_bounds(command.bounds, points_to_world),
                    tint,
                    ImageUvRect::default(),
                    draw_depth,
                ),
            ))
        })
        .collect()
}

/// Applies one widget visual-slot override to a routed image record.
///
/// Runs inside `collect_panel_image_records`, so a replacement `texture`
/// changes the selected `ImageBatchKey` and `ImageBatchStore::upsert_record`
/// moves the record to the destination batch, while a `color` or `offset`
/// patches only the record's `tint`/`local_transform` in its current batch.
fn apply_image_visual_override(
    slot_override: Option<&VisualSlotOverride>,
    texture: Handle<Image>,
    tint: Vec4,
    local_transform: Transform,
) -> (Handle<Image>, Vec4, Transform) {
    let Some(slot_override) = slot_override else {
        return (texture, tint, local_transform);
    };
    let texture = slot_override.texture.clone().unwrap_or(texture);
    let tint = slot_override.tint.map_or(tint, linear_tint);
    let mut local_transform = local_transform;
    if let Some(offset) = slot_override.offset {
        local_transform.translation += offset.extend(0.0);
    }
    (texture, tint, local_transform)
}

fn image_record_source(
    command: &RenderCommand,
    precompose_cache: &PanelPrecomposeCache,
) -> Option<(Handle<Image>, Vec4)> {
    match &command.kind {
        RenderCommandKind::Image { handle, tint } => Some((handle.clone(), linear_tint(*tint))),
        RenderCommandKind::PrecomposeLdr => precompose_cache
            .entry(command.element_idx)
            .map(|entry| (entry.image.clone(), linear_tint(Color::WHITE))),
        _ => None,
    }
}

fn image_size_from_bounds(bounds: BoundingBox, points_to_world: f32) -> Vec2 {
    Vec2::new(
        bounds.width * points_to_world,
        bounds.height * points_to_world,
    )
}

fn local_transform_from_bounds(
    bounds: BoundingBox,
    points_to_world: f32,
    anchor_x: f32,
    anchor_y: f32,
) -> Transform {
    let size = image_size_from_bounds(bounds, points_to_world);
    let local_x = bounds.x.mul_add(points_to_world, size.x * 0.5) - anchor_x;
    let local_y = -(bounds.y.mul_add(points_to_world, size.y * 0.5) - anchor_y);
    Transform::from_xyz(local_x, local_y, TEXT_Z_OFFSET)
}

fn linear_tint(color: Color) -> Vec4 {
    let linear = color.to_linear();
    Vec4::new(linear.red, linear.green, linear.blue, linear.alpha)
}

fn reconcile_image_batch_entities(
    mut store: ResMut<ImageBatchStore>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ImageExtendedMaterial>>,
    mut storage_buffers: ResMut<Assets<ShaderBuffer>>,
    mut commands: Commands,
) {
    for entity in store.take_empty_batches() {
        commands.entity(entity).despawn();
    }

    for (key, batch) in store.batches_mut() {
        if batch.is_empty() {
            continue;
        }
        if batch.entity.is_some() {
            grow_image_batch_resources(
                key,
                batch,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut storage_buffers,
            );
        } else {
            spawn_image_batch_entity(
                key,
                batch,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut storage_buffers,
            );
        }
    }
}

fn commit_image_batch_buffers(
    mut store: ResMut<ImageBatchStore>,
    mut staged_uploads: ResMut<RetainedBatchBufferUploads>,
    mut perf: ResMut<DiegeticPerfStats>,
) {
    perf.image_breakdown.clear();
    for (key, batch) in store.batches_mut() {
        perf.image_breakdown
            .push(image_batch_summary(key, batch.record_count()));
        let _ = commit_image_batch_records(batch, &mut staged_uploads);
    }
}

/// Builds the per-batch summary for the batch-validation diagnostic. Image
/// batches carry no `PipelineCompatibility`/`ResourceCompatibility`, so this
/// fills [`BatchSummary`] from the image key directly: every image batch is
/// `Blend`, unlit, and textured; it splits by render layer, shadow, and z-index.
fn image_batch_summary(key: &ImageBatchKey, record_count: u32) -> BatchSummary {
    BatchSummary {
        z_index:       key.z_index,
        render_layers: key.layers.0.iter().map(usize::to_u32).collect(),
        casts_shadow:  matches!(key.shadow, VisualShadow::Cast),
        unlit:         true,
        alpha_mode:    "Blend".to_owned(),
        textured:      true,
        record_count:  record_count.to_usize().into(),
    }
}

fn spawn_image_batch_entity(
    key: &ImageBatchKey,
    batch: &mut ImageBatch,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ImageExtendedMaterial>,
    storage_buffers: &mut Assets<ShaderBuffer>,
) {
    allocate_image_batch_resources(key, batch, meshes, materials, storage_buffers);
    let Some(gpu) = batch.gpu.as_ref() else {
        return;
    };
    let mut entity = commands.spawn((
        DiegeticImageBatch,
        Mesh3d(gpu.mesh.clone()),
        MeshMaterial3d(gpu.material.clone()),
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
}

/// Allocates an image batch record buffer, inert mesh, and material.
fn allocate_image_batch_resources(
    key: &ImageBatchKey,
    batch: &mut ImageBatch,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ImageExtendedMaterial>,
    storage_buffers: &mut Assets<ShaderBuffer>,
) {
    let capacity = image_record_capacity(batch.record_count());
    let records = storage_buffers.add(ShaderBuffer::from(padded_image_render_records(
        batch.records(),
        batch.first_draw_order_index(),
        capacity,
    )));
    let mesh = meshes.add(inert_image_batch_mesh(capacity));
    let material = materials.add(image_material::image_batch_material(key, records.clone()));
    batch.gpu = Some(ImageBatchResources {
        records,
        mesh,
        material,
        capacity,
    });
    batch.record_upload.clear();
}

/// Grows one image batch's record buffer to fit its live records.
fn grow_image_batch_resources(
    key: &ImageBatchKey,
    batch: &mut ImageBatch,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ImageExtendedMaterial>,
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
    let records = storage_buffers.add(ShaderBuffer::from(padded_image_render_records(
        batch.records(),
        batch.first_draw_order_index(),
        capacity,
    )));
    let mesh = meshes.add(inert_image_batch_mesh(capacity));
    commands.entity(entity).insert(Mesh3d(mesh.clone()));
    let Some(gpu) = batch.gpu.as_mut() else {
        return;
    };
    if let Some(mut material) = materials.get_mut(&gpu.material) {
        image_material::set_image_material_records(&mut material, records.clone());
    } else {
        let material = materials.add(image_material::image_batch_material(key, records.clone()));
        commands
            .entity(entity)
            .insert(MeshMaterial3d(material.clone()));
        gpu.material = material;
    }
    gpu.records = records;
    gpu.mesh = mesh;
    gpu.capacity = capacity;
    batch.record_upload.clear();
}

/// Uploads a dirty image record buffer with a fixed-capacity payload.
fn commit_image_batch_records(
    batch: &mut ImageBatch,
    staged_uploads: &mut RetainedBatchBufferUploads,
) -> Option<usize> {
    if !batch.record_upload.is_set() {
        return None;
    }
    let gpu = batch.gpu.as_ref()?;
    let payload = padded_image_render_records(
        batch.records(),
        batch.first_draw_order_index(),
        gpu.capacity,
    );
    let byte_len = image_record_payload_bytes(gpu.capacity);
    let records = gpu.records.clone();
    batch.record_upload.clear();
    material_table::stage_retained_batch_buffer_upload(
        staged_uploads,
        &records,
        ShaderBuffer::from(payload),
    );
    Some(byte_len)
}

fn padded_image_render_records(
    records: &[ResolvedImageRecord],
    first_draw_order_index: DrawOrderIndex,
    capacity: u32,
) -> Vec<ImageRenderRecord> {
    let mut padded = Vec::with_capacity(capacity.to_usize());
    padded.extend(
        records
            .iter()
            .map(|record| ImageRenderRecord::from_resolved(record, first_draw_order_index)),
    );
    padded.resize(
        capacity.to_usize().max(records.len()),
        ImageRenderRecord::padded(),
    );
    padded
}

fn image_record_capacity(record_count: u32) -> u32 { record_count.max(1).next_power_of_two() }

fn image_record_payload_bytes(capacity: u32) -> usize {
    let record_bytes = usize::try_from(ImageRenderRecord::SHADER_SIZE.get()).unwrap_or(usize::MAX);
    record_bytes.saturating_mul(capacity.to_usize())
}

const fn image_record_transform(record: &ResolvedImageRecord) -> Mat4 { record.transform }

fn inert_image_batch_mesh(capacity: u32) -> Mesh {
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

fn centered_corners(size: Vec2) -> [Vec2; 4] {
    let half_size = size * 0.5;
    [
        Vec2::new(-half_size.x, half_size.y),
        Vec2::new(half_size.x, half_size.y),
        Vec2::new(half_size.x, -half_size.y),
        Vec2::new(-half_size.x, -half_size.y),
    ]
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::asset::AssetPlugin;
    use bevy::camera::visibility::RenderLayers;
    use bevy::prelude::AssetApp;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::TransformPlugin;
    use bevy::prelude::UVec2;
    use bevy::prelude::With;

    use super::*;
    use crate::Anchor;
    use crate::cascade::Resolved;
    use crate::layout::BoundingBox;
    use crate::layout::LayoutResult;
    use crate::layout::Pt;
    use crate::layout::RectangleSource;
    use crate::layout::RenderCommand;
    use crate::layout::RenderCommandKind;
    use crate::layout::ShadowCasting;
    use crate::panel::ComputedDiegeticPanel;
    use crate::panel::DiegeticPanel;
    use crate::panel::PanelPrecomposeCache;
    use crate::panel::PrecomposeCacheEntry;
    use crate::render::draw_order::DrawOrder;
    use crate::render::image_material;

    const FIRST_COMMAND_INDEX: usize = 0;
    const FIRST_PANEL_TRANSLATION: Vec3 = Vec3::new(1.0, 2.0, 3.0);
    const GROWN_RECORD_COUNT: u32 = 2;
    const INITIAL_CAPACITY: u32 = 1;
    const NONZERO_BOUNDS_HEIGHT: f32 = 10.0;
    const NONZERO_BOUNDS_WIDTH: f32 = 20.0;
    const NONZERO_BOUNDS_X: f32 = 10.0;
    const NONZERO_BOUNDS_Y: f32 = 5.0;
    const OUTSIDE_PANEL_X: f32 = 150.0;
    const PANEL_HEIGHT_PT: f32 = 50.0;
    const PANEL_WORLD_HEIGHT: f32 = 0.5;
    const PANEL_WIDTH_PT: f32 = 100.0;
    const PRECOMPOSE_CAMERA_BITS: u64 = 12;
    const PRECOMPOSE_HELPER_BITS: u64 = 11;
    const SECOND_PANEL_TRANSLATION: Vec3 = Vec3::new(-2.0, 0.5, 0.25);
    const TEST_CAPACITY: u32 = 8;
    const TEST_PANEL_BITS: u64 = 1;
    const TEST_BATCH_ENTITY_BITS: u64 = 10;
    const TEST_BOUNDS_SIZE: f32 = 10.0;
    const TEST_EPSILON: f32 = 0.0001;
    const UPDATED_TINT: Vec4 = Vec4::new(0.25, 0.5, 0.75, 1.0);
    const WORLD_BOUNDS_MAX: Vec3 = Vec3::new(4.0, 7.0, 7.0);
    const WORLD_BOUNDS_MIN: Vec3 = Vec3::new(2.0, 3.0, 7.0);
    const WORLD_BOUNDS_RECORD_SIZE: Vec2 = Vec2::new(2.0, 4.0);
    const WORLD_BOUNDS_RECORD_TRANSLATION: Vec3 = Vec3::new(3.0, 5.0, 7.0);

    #[test]
    fn compatible_records_share_one_batch() {
        let mut store = ImageBatchStore::default();
        let mut images = Assets::<Image>::default();
        let key = test_batch_key(images.add(Image::default()));
        store.upsert_record(key.clone(), test_record(0, Vec4::ONE));
        store.upsert_record(key.clone(), test_record(1, UPDATED_TINT));

        let batches: Vec<_> = store.batches().collect();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].0, &key);
        assert_eq!(batches[0].1.records().len(), 2);
        assert_eq!(batches[0].1.records()[0].tint, Vec4::ONE);
        assert_eq!(batches[0].1.records()[1].tint, UPDATED_TINT);
    }

    #[test]
    fn incompatible_key_fields_split_batches() {
        let mut images = Assets::<Image>::default();
        let texture = images.add(Image::default());
        let base_key = test_batch_key(texture);
        let variants = [
            test_batch_key(images.add(Image::default())),
            ImageBatchKey {
                layers: BatchRenderLayers(RenderLayers::layer(1)),
                ..base_key.clone()
            },
            ImageBatchKey {
                shadow: VisualShadow::None,
                ..base_key.clone()
            },
            ImageBatchKey {
                z_index_rank: DrawZIndexRank::from(1),
                ..base_key.clone()
            },
        ];

        for variant in variants {
            let mut store = ImageBatchStore::default();
            store.upsert_record(base_key.clone(), test_record(0, Vec4::ONE));
            store.upsert_record(variant, test_record(1, Vec4::ONE));

            let non_empty_batches = store
                .batches()
                .filter(|(_, batch)| !batch.is_empty())
                .count();
            assert_eq!(non_empty_batches, 2);
        }
    }

    #[test]
    fn moving_record_between_keys_splits_and_merges() {
        let mut images = Assets::<Image>::default();
        let base_key = test_batch_key(images.add(Image::default()));
        let other_key = test_batch_key(images.add(Image::default()));
        let mut store = ImageBatchStore::default();
        let record = test_record(0, Vec4::ONE);

        store.upsert_record(base_key.clone(), record.clone());
        store.upsert_record(other_key.clone(), record.clone());
        store.take_empty_batches();

        assert!(store.get_mut(&base_key).is_none());
        assert_eq!(
            store
                .get_mut(&other_key)
                .expect("moved record creates other batch")
                .records()
                .len(),
            1
        );

        store.upsert_record(base_key.clone(), record);
        store.take_empty_batches();

        assert!(store.get_mut(&other_key).is_none());
        assert_eq!(
            store
                .get_mut(&base_key)
                .expect("record moves back to base batch")
                .records()
                .len(),
            1
        );
    }

    #[test]
    fn record_updates_keep_batch_entity_and_buffer() {
        let mut images = Assets::<Image>::default();
        let key = test_batch_key(images.add(Image::default()));
        let mut store = ImageBatchStore::default();
        let mut meshes = Assets::<Mesh>::default();
        let mut materials = Assets::<ImageExtendedMaterial>::default();
        let mut storage_buffers = Assets::<ShaderBuffer>::default();
        let mut staged_uploads = RetainedBatchBufferUploads::default();
        store.upsert_record(key.clone(), test_record(0, Vec4::ONE));

        let entity = Entity::from_bits(TEST_BATCH_ENTITY_BITS);
        let initial_records = {
            let batch = store.get_mut(&key).expect("batch exists after insert");
            batch.entity = Some(entity);
            allocate_image_batch_resources(
                &key,
                batch,
                &mut meshes,
                &mut materials,
                &mut storage_buffers,
            );
            batch
                .gpu
                .as_ref()
                .expect("allocation installs resources")
                .records
                .clone()
        };

        store.upsert_record(key.clone(), test_record(0, UPDATED_TINT));
        let batch = store.get_mut(&key).expect("updated batch still exists");
        assert_eq!(batch.entity, Some(entity));
        assert_eq!(
            batch
                .gpu
                .as_ref()
                .expect("same-key update keeps resources")
                .records,
            initial_records
        );
        assert_eq!(
            commit_image_batch_records(batch, &mut staged_uploads),
            Some(image_record_payload_bytes(INITIAL_CAPACITY))
        );
        assert!(staged_uploads.contains(&initial_records));
        assert_eq!(
            batch.gpu.as_ref().expect("commit keeps resources").records,
            initial_records
        );
    }

    #[test]
    fn growth_uses_fixed_capacity_payloads_and_repoints_material_binding() {
        let mut app = image_batch_app();
        let mut images = Assets::<Image>::default();
        let texture = images.add(Image::default());
        let panel = spawn_panel(
            &mut app,
            vec![image_command(texture.clone(), Color::WHITE, 0)],
        );

        app.update();
        let initial = single_batch_resources(&app);
        assert_eq!(initial.capacity, INITIAL_CAPACITY);

        set_panel_commands(
            &mut app,
            panel,
            vec![
                image_command(texture.clone(), Color::WHITE, 0),
                image_command(texture, Color::WHITE, 1),
            ],
        );
        app.update();

        let grown = single_batch_resources(&app);
        assert_eq!(grown.capacity, image_record_capacity(GROWN_RECORD_COUNT));
        assert_ne!(grown.records, initial.records);
        assert_ne!(grown.mesh, initial.mesh);
        assert_eq!(grown.material, initial.material);
        let materials = app.world().resource::<Assets<ImageExtendedMaterial>>();
        let material = materials
            .get(&grown.material)
            .expect("grown image batch material should exist");
        assert_eq!(
            image_material::image_material_records(material),
            &grown.records
        );

        for count in 0..=TEST_CAPACITY.to_usize() {
            assert_eq!(
                padded_image_render_records(
                    &vec![test_record(0, Vec4::ONE); count],
                    DrawOrderIndex::default(),
                    TEST_CAPACITY,
                )
                .len(),
                TEST_CAPACITY.to_usize(),
                "image payload length must equal capacity at {count} records"
            );
        }
    }

    #[test]
    fn sort_records_orders_by_draw_order_index() {
        let mut batch = ImageBatch::default();
        batch.upsert_record(test_record(1, Vec4::ONE));
        batch.upsert_record(test_record(0, Vec4::ONE));

        let command_indices: Vec<CommandIndex> = batch
            .records()
            .iter()
            .map(|record| record.record_key.command_index)
            .collect();
        assert_eq!(
            command_indices,
            vec![
                CommandIndex::from(FIRST_COMMAND_INDEX),
                CommandIndex::from(1)
            ]
        );
        assert_eq!(
            batch.first_draw_order_index(),
            test_draw_depth(FIRST_COMMAND_INDEX).draw_order_index_value()
        );
    }

    #[test]
    fn upsert_carries_transform_and_skips_dirty_flags_for_unchanged_record() {
        let mut images = Assets::<Image>::default();
        let key = test_batch_key(images.add(Image::default()));
        let mut store = ImageBatchStore::default();
        let record = test_record(0, Vec4::ONE);

        store.upsert_record(key.clone(), record.clone());
        let retained_transform = Mat4::from_translation(Vec3::new(2.0, 3.0, 4.0));
        {
            let batch = store.get_mut(&key).expect("batch exists after insert");
            batch.records[0].transform = retained_transform;
            batch.record_upload.clear();
            batch.bounds_update.clear();
        }

        store.upsert_record(key.clone(), record);

        let batch = store
            .get_mut(&key)
            .expect("batch remains after no-op upsert");
        assert_eq!(batch.records()[0].transform, retained_transform);
        assert!(!batch.record_upload.is_set());
        assert!(!batch.bounds_update.is_set());
    }

    #[test]
    fn router_groups_compatible_image_commands_by_texture() {
        let mut app = image_batch_app();
        let mut images = Assets::<Image>::default();
        let texture = images.add(Image::default());
        spawn_panel(
            &mut app,
            vec![
                image_command(texture.clone(), Color::WHITE, 0),
                image_command(texture.clone(), Color::srgb(0.25, 0.5, 0.75), 1),
            ],
        );

        app.update();

        let (key, records) = single_image_batch(&app);
        assert_eq!(key.texture, texture);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].record_key.command_index, CommandIndex::from(0));
        assert_eq!(records[1].record_key.command_index, CommandIndex::from(1));
    }

    #[test]
    fn router_splits_records_by_texture_handle() {
        let mut app = image_batch_app();
        let mut images = Assets::<Image>::default();
        let first_texture = images.add(Image::default());
        let second_texture = images.add(Image::default());
        spawn_panel(
            &mut app,
            vec![
                image_command(first_texture, Color::WHITE, 0),
                image_command(second_texture, Color::WHITE, 1),
            ],
        );

        app.update();

        assert_eq!(non_empty_image_batch_count(&app), 2);
    }

    #[test]
    fn router_rekeys_on_layer_and_shadow_changes() {
        let mut app = image_batch_app();
        let mut images = Assets::<Image>::default();
        let texture = images.add(Image::default());
        let panel = spawn_panel(&mut app, vec![image_command(texture, Color::WHITE, 0)]);

        app.update();
        let (initial_key, _) = single_image_batch(&app);
        assert_eq!(
            initial_key.layers,
            BatchRenderLayers(RenderLayers::layer(0))
        );
        assert_eq!(initial_key.shadow, VisualShadow::Cast);

        app.world_mut()
            .entity_mut(panel)
            .insert((RenderLayers::layer(2), Resolved(ShadowCasting::Off)));
        app.update();

        let (updated_key, records) = single_image_batch(&app);
        assert_eq!(
            updated_key.layers,
            BatchRenderLayers(RenderLayers::layer(2))
        );
        assert_eq!(updated_key.shadow, VisualShadow::None);
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn router_culls_empty_clip_commands() {
        let mut app = image_batch_app();
        let mut images = Assets::<Image>::default();
        let texture = images.add(Image::default());
        spawn_panel(
            &mut app,
            vec![image_command_with_bounds(
                texture,
                Color::WHITE,
                0,
                BoundingBox {
                    x:      OUTSIDE_PANEL_X,
                    y:      0.0,
                    width:  TEST_BOUNDS_SIZE,
                    height: TEST_BOUNDS_SIZE,
                },
            )],
        );

        app.update();

        assert!(image_records(&app).is_empty());
        assert_eq!(non_empty_image_batch_count(&app), 0);
    }

    #[test]
    fn router_uses_precompose_cache_image_and_white_tint() {
        let mut app = image_batch_app();
        let mut images = Assets::<Image>::default();
        let texture = images.add(Image::default());
        let panel = spawn_panel(&mut app, vec![precompose_command(0)]);
        insert_precompose_entry(&mut app, panel, 0, texture.clone());

        app.update();

        let (key, records) = single_image_batch(&app);
        assert_eq!(key.texture, texture);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tint, linear_tint(Color::WHITE));
    }

    #[test]
    fn router_skips_absent_precompose_cache_entries() {
        let mut app = image_batch_app();
        spawn_panel(&mut app, vec![precompose_command(0)]);

        app.update();

        assert!(image_records(&app).is_empty());
        assert_eq!(non_empty_image_batch_count(&app), 0);
    }

    #[test]
    fn router_merges_cross_panel_same_texture_records() {
        let mut app = image_batch_app();
        let mut images = Assets::<Image>::default();
        let texture = images.add(Image::default());
        let first_panel = spawn_panel(
            &mut app,
            vec![image_command(texture.clone(), Color::WHITE, 0)],
        );
        let second_panel = spawn_panel(
            &mut app,
            vec![image_command(texture.clone(), Color::WHITE, 0)],
        );

        app.update();

        let (key, records) = single_image_batch(&app);
        assert_eq!(key.texture, texture);
        assert_eq!(records.len(), 2);
        assert!(
            records
                .iter()
                .any(|record| record.record_key.panel == first_panel)
        );
        assert!(
            records
                .iter()
                .any(|record| record.record_key.panel == second_panel)
        );
    }

    #[test]
    fn reconcile_spawns_and_despawns_batch_entity_for_key_lifecycle() {
        let mut app = image_batch_app();
        let mut images = Assets::<Image>::default();
        let texture = images.add(Image::default());
        let panel = spawn_panel(&mut app, vec![image_command(texture, Color::WHITE, 0)]);

        app.update();

        assert_eq!(image_batch_entity_count(&mut app), 1);
        assert!(single_batch_resources(&app).capacity >= INITIAL_CAPACITY);

        set_panel_commands(&mut app, panel, Vec::new());
        app.update();

        assert_eq!(image_batch_entity_count(&mut app), 0);
        assert_eq!(non_empty_image_batch_count(&app), 0);
    }

    #[test]
    fn image_batch_entities_ignore_stock_mesh_picking() {
        let mut app = image_batch_app();
        let mut images = Assets::<Image>::default();
        let texture = images.add(Image::default());
        spawn_panel(&mut app, vec![image_command(texture, Color::WHITE, 0)]);

        app.update();

        let mut query = app
            .world_mut()
            .query_filtered::<(Entity, &Pickable), With<DiegeticImageBatch>>();
        let batches = query.iter(app.world()).collect::<Vec<_>>();
        assert_eq!(batches.len(), 1);
        assert!(!batches[0].1.is_hoverable);
        assert!(!batches[0].1.should_block_lower);
    }

    #[test]
    fn cross_panel_same_texture_records_use_each_panel_transform() {
        let mut app = image_batch_app();
        let mut images = Assets::<Image>::default();
        let texture = images.add(Image::default());
        let command = image_command(texture, Color::WHITE, 0);
        let first_panel = spawn_panel_with_anchor_transform(
            &mut app,
            vec![command.clone()],
            Anchor::TopLeft,
            Transform::from_translation(FIRST_PANEL_TRANSLATION),
        );
        let second_panel = spawn_panel_with_anchor_transform(
            &mut app,
            vec![command],
            Anchor::TopLeft,
            Transform::from_translation(SECOND_PANEL_TRANSLATION),
        );

        app.update();

        let (_, records) = single_image_batch(&app);
        assert_eq!(records.len(), 2);
        assert_record_translation(
            record_for_panel(&records, first_panel),
            expected_world_translation(&app, first_panel, image_bounds()),
        );
        assert_record_translation(
            record_for_panel(&records, second_panel),
            expected_world_translation(&app, second_panel, image_bounds()),
        );
    }

    #[test]
    fn world_bounds_cover_transformed_image_record_quads() {
        let mut batch = ImageBatch::default();
        let mut record = test_record(0, Vec4::ONE);
        record.size = WORLD_BOUNDS_RECORD_SIZE;
        record.transform = Mat4::from_translation(WORLD_BOUNDS_RECORD_TRANSLATION);
        batch.upsert_record(record);

        let (min, max) = batch
            .world_bounds()
            .expect("image batch should have bounds");

        assert_vec3_close(min, WORLD_BOUNDS_MIN);
        assert_vec3_close(max, WORLD_BOUNDS_MAX);
    }

    #[test]
    fn static_batch_does_not_reupload_across_frames() {
        let mut app = image_batch_app();
        let mut images = Assets::<Image>::default();
        let texture = images.add(Image::default());
        spawn_panel(&mut app, vec![image_command(texture, Color::WHITE, 0)]);

        app.update();
        let initial = single_batch_resources(&app);
        assert_single_batch_not_dirty(&app);

        app.update();

        let stable = single_batch_resources(&app);
        assert_eq!(stable.records, initial.records);
        assert_eq!(stable.mesh, initial.mesh);
        assert_eq!(stable.material, initial.material);
        assert_single_batch_not_dirty(&app);
    }

    #[test]
    fn batched_image_matches_legacy_entity_coordinate_conversion() {
        let mut app = image_batch_app();
        let mut images = Assets::<Image>::default();
        let texture = images.add(Image::default());
        let bounds = nonzero_image_bounds();
        let panel = spawn_panel_with_anchor_transform(
            &mut app,
            vec![image_command_with_bounds(texture, Color::WHITE, 0, bounds)],
            Anchor::Center,
            Transform::from_translation(FIRST_PANEL_TRANSLATION),
        );

        app.update();

        let (_, records) = single_image_batch(&app);
        let record = record_for_panel(&records, panel);
        let (expected_size, expected_local_transform) =
            expected_legacy_image_geometry(&app, panel, bounds);
        assert_vec2_close(record.size, expected_size);
        assert_vec3_close(
            record.local_transform.translation,
            expected_local_transform.translation,
        );
        assert_mat4_close(
            record.transform,
            Transform::from_translation(FIRST_PANEL_TRANSLATION).to_matrix()
                * expected_local_transform.to_matrix(),
        );
    }

    #[test]
    fn image_batch_key_derives_depth_bias_from_rank() {
        let mut images = Assets::<Image>::default();
        let key = ImageBatchKey {
            z_index_rank: DrawZIndexRank::from(1),
            ..test_batch_key(images.add(Image::default()))
        };

        assert_eq!(
            key.depth_bias().to_bits(),
            DrawZIndexRank::from(1).screen_depth_bias().get().to_bits()
        );
    }

    fn test_batch_key(texture: Handle<Image>) -> ImageBatchKey {
        ImageBatchKey {
            texture,
            layers: BatchRenderLayers(RenderLayers::layer(0)),
            shadow: VisualShadow::Cast,
            z_index: DrawZIndex(0),
            z_index_rank: DrawZIndexRank::default(),
        }
    }

    fn image_batch_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(TransformPlugin)
            .add_plugins(AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<ShaderBuffer>()
            // `commit_image_batch_buffers` stages into the buffer-upload queue
            // `MaterialTablePlugin` owns, which this app does not add.
            .init_resource::<RetainedBatchBufferUploads>()
            .add_plugins(ImageBatchPlugin);
        app
    }

    fn spawn_panel(app: &mut App, commands: Vec<RenderCommand>) -> Entity {
        spawn_panel_with_anchor_transform(app, commands, Anchor::TopLeft, Transform::default())
    }

    fn spawn_panel_with_anchor_transform(
        app: &mut App,
        commands: Vec<RenderCommand>,
        anchor: Anchor,
        transform: Transform,
    ) -> Entity {
        let panel = test_panel(anchor);
        let computed = computed_panel(commands);
        app.world_mut()
            .spawn((
                panel,
                computed,
                PanelPrecomposeCache::default(),
                Visibility::Visible,
                transform,
                GlobalTransform::default(),
            ))
            .id()
    }

    fn test_panel(anchor: Anchor) -> DiegeticPanel {
        DiegeticPanel::world()
            .anchor(anchor)
            .size(Pt(PANEL_WIDTH_PT), Pt(PANEL_HEIGHT_PT))
            .world_height(PANEL_WORLD_HEIGHT)
            .build()
            .expect("test panel should build")
    }

    fn computed_panel(commands: Vec<RenderCommand>) -> ComputedDiegeticPanel {
        let mut result = LayoutResult::default();
        result.commands = commands;
        let mut computed = ComputedDiegeticPanel::default();
        computed.set_result(result);
        computed
    }

    fn set_panel_commands(app: &mut App, panel: Entity, commands: Vec<RenderCommand>) {
        app.world_mut()
            .get_mut::<ComputedDiegeticPanel>(panel)
            .expect("panel should have computed layout")
            .set_result(layout_result(commands));
    }

    fn layout_result(commands: Vec<RenderCommand>) -> LayoutResult {
        let mut result = LayoutResult::default();
        result.commands = commands;
        result
    }

    fn insert_precompose_entry(
        app: &mut App,
        panel: Entity,
        element_idx: usize,
        texture: Handle<Image>,
    ) {
        app.world_mut()
            .get_mut::<PanelPrecomposeCache>(panel)
            .expect("panel should have a precompose cache")
            .entries_mut()
            .insert(
                element_idx,
                PrecomposeCacheEntry {
                    image:        texture,
                    helper_panel: Entity::from_bits(PRECOMPOSE_HELPER_BITS),
                    camera:       Entity::from_bits(PRECOMPOSE_CAMERA_BITS),
                    pixel_size:   UVec2::ONE,
                },
            );
    }

    fn image_command(handle: Handle<Image>, tint: Color, element_idx: usize) -> RenderCommand {
        image_command_with_bounds(
            handle,
            tint,
            element_idx,
            BoundingBox {
                x:      0.0,
                y:      0.0,
                width:  TEST_BOUNDS_SIZE,
                height: TEST_BOUNDS_SIZE,
            },
        )
    }

    fn image_command_with_bounds(
        handle: Handle<Image>,
        tint: Color,
        element_idx: usize,
        bounds: BoundingBox,
    ) -> RenderCommand {
        RenderCommand {
            bounds,
            kind: RenderCommandKind::Image { handle, tint },
            element_idx,
            z_index: DrawZIndex(0),
        }
    }

    fn precompose_command(element_idx: usize) -> RenderCommand {
        RenderCommand {
            bounds: BoundingBox {
                x:      0.0,
                y:      0.0,
                width:  TEST_BOUNDS_SIZE,
                height: TEST_BOUNDS_SIZE,
            },
            kind: RenderCommandKind::PrecomposeLdr,
            element_idx,
            z_index: DrawZIndex(0),
        }
    }

    #[derive(Debug)]
    struct BatchResourceHandles {
        records:  Handle<ShaderBuffer>,
        mesh:     Handle<Mesh>,
        material: Handle<ImageExtendedMaterial>,
        capacity: u32,
    }

    fn single_batch_resources(app: &App) -> BatchResourceHandles {
        let batches: Vec<_> = app
            .world()
            .resource::<ImageBatchStore>()
            .batches()
            .filter(|(_, batch)| !batch.is_empty())
            .collect();
        assert_eq!(batches.len(), 1, "expected exactly one image batch");
        let gpu = batches[0]
            .1
            .gpu
            .as_ref()
            .expect("batch should have GPU assets");
        BatchResourceHandles {
            records:  gpu.records.clone(),
            mesh:     gpu.mesh.clone(),
            material: gpu.material.clone(),
            capacity: gpu.capacity,
        }
    }

    fn single_image_batch(app: &App) -> (ImageBatchKey, Vec<ResolvedImageRecord>) {
        let batches: Vec<_> = app
            .world()
            .resource::<ImageBatchStore>()
            .batches()
            .filter(|(_, batch)| !batch.is_empty())
            .collect();
        assert_eq!(batches.len(), 1, "expected exactly one image batch");
        (batches[0].0.clone(), batches[0].1.records().to_vec())
    }

    fn non_empty_image_batch_count(app: &App) -> usize {
        app.world()
            .resource::<ImageBatchStore>()
            .batches()
            .filter(|(_, batch)| !batch.is_empty())
            .count()
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
                .panel
                .to_bits()
                .cmp(&right.record_key.panel.to_bits())
                .then(
                    left.record_key
                        .command_index
                        .cmp(&right.record_key.command_index),
                )
        });
        records
    }

    fn image_batch_entity_count(app: &mut App) -> usize {
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<DiegeticImageBatch>>();
        query.iter(app.world()).count()
    }

    fn assert_single_batch_not_dirty(app: &App) {
        let batches: Vec<_> = app
            .world()
            .resource::<ImageBatchStore>()
            .batches()
            .filter(|(_, batch)| !batch.is_empty())
            .collect();
        assert_eq!(batches.len(), 1, "expected exactly one image batch");
        assert!(!batches[0].1.record_upload.is_set());
        assert!(!batches[0].1.bounds_update.is_set());
    }

    fn image_bounds() -> BoundingBox {
        BoundingBox {
            x:      0.0,
            y:      0.0,
            width:  TEST_BOUNDS_SIZE,
            height: TEST_BOUNDS_SIZE,
        }
    }

    fn nonzero_image_bounds() -> BoundingBox {
        BoundingBox {
            x:      NONZERO_BOUNDS_X,
            y:      NONZERO_BOUNDS_Y,
            width:  NONZERO_BOUNDS_WIDTH,
            height: NONZERO_BOUNDS_HEIGHT,
        }
    }

    fn record_for_panel(records: &[ResolvedImageRecord], panel: Entity) -> &ResolvedImageRecord {
        records
            .iter()
            .find(|record| record.record_key.panel == panel)
            .expect("panel should have an image record")
    }

    fn expected_world_translation(app: &App, panel: Entity, bounds: BoundingBox) -> Vec3 {
        let (_, local_transform) = expected_legacy_image_geometry(app, panel, bounds);
        let panel_transform = app
            .world()
            .get::<GlobalTransform>(panel)
            .expect("panel should have a global transform");
        (panel_transform.to_matrix() * local_transform.to_matrix()).transform_point3(Vec3::ZERO)
    }

    fn expected_legacy_image_geometry(
        app: &App,
        panel: Entity,
        bounds: BoundingBox,
    ) -> (Vec2, Transform) {
        let panel = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("panel should have a DiegeticPanel component");
        let points_to_world = panel.points_to_world();
        let (anchor_x, anchor_y) = panel.anchor_offsets();
        let width = bounds.width * points_to_world;
        let height = bounds.height * points_to_world;
        let x = bounds.x.mul_add(points_to_world, width * 0.5) - anchor_x;
        let y = -(bounds.y.mul_add(points_to_world, height * 0.5) - anchor_y);
        (
            Vec2::new(width, height),
            Transform::from_xyz(x, y, TEXT_Z_OFFSET),
        )
    }

    fn assert_record_translation(record: &ResolvedImageRecord, expected: Vec3) {
        assert_vec3_close(record.transform.transform_point3(Vec3::ZERO), expected);
    }

    fn assert_vec2_close(actual: Vec2, expected: Vec2) {
        assert!((actual.x - expected.x).abs() <= TEST_EPSILON);
        assert!((actual.y - expected.y).abs() <= TEST_EPSILON);
    }

    fn assert_vec3_close(actual: Vec3, expected: Vec3) {
        assert!((actual.x - expected.x).abs() <= TEST_EPSILON);
        assert!((actual.y - expected.y).abs() <= TEST_EPSILON);
        assert!((actual.z - expected.z).abs() <= TEST_EPSILON);
    }

    fn assert_mat4_close(actual: Mat4, expected: Mat4) {
        for (index, (actual, expected)) in actual
            .to_cols_array()
            .into_iter()
            .zip(expected.to_cols_array())
            .enumerate()
        {
            assert!(
                (actual - expected).abs() <= TEST_EPSILON,
                "matrix value {index} differs: actual {actual}, expected {expected}"
            );
        }
    }

    fn test_record(command_index: usize, tint: Vec4) -> ResolvedImageRecord {
        ResolvedImageRecord::new(
            ImageRecordKey {
                panel:         Entity::from_bits(TEST_PANEL_BITS),
                command_index: CommandIndex::from(command_index),
            },
            Transform::IDENTITY,
            Vec2::ONE,
            tint,
            ImageUvRect::default(),
            test_draw_depth(command_index),
        )
    }

    fn test_draw_depth(index: usize) -> DrawCommandDepth {
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
        DrawOrder::from_commands(&commands)
            .depth_for(index)
            .expect("draw command should have depth")
    }
}
