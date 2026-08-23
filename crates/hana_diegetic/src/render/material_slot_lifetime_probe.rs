use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;

use bevy::prelude::App;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::MinimalPlugins;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::SystemSet;
use bevy::prelude::Update;
use bevy::prelude::World;
use hana_kana::ToU32;
use hana_kana::ToUsize;

const STRESS_SLOT_COUNT: usize = 128;
const STRESS_FRAME_COUNT: usize = 12;
const TWO_FRAME_RETIREMENT: usize = 2;
const TWO_FRAME_RETIREMENT_U64: u64 = 2;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct MaterialSlotId(u32);

impl MaterialSlotId {
    fn index(self) -> usize { self.0.to_usize() }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct MaterialSlotGeneration(u32);

impl MaterialSlotGeneration {
    fn next(self) -> Self { Self(self.0 + 1) }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct MaterialSlotRef {
    id:         MaterialSlotId,
    generation: MaterialSlotGeneration,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum SlotPointer {
    Id(MaterialSlotId),
    Ref(MaterialSlotRef),
}

impl SlotPointer {
    fn id(self) -> MaterialSlotId {
        match self {
            Self::Id(id) => id,
            Self::Ref(reference) => reference.id,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum SourceOwner {
    Sdf(Entity),
    Text(Entity),
    PanelShape(Entity),
}

impl Default for SourceOwner {
    fn default() -> Self { Self::Sdf(Entity::PLACEHOLDER) }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SdfSurfaceMaterialKey {
    panel:         Entity,
    command_index: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TextRunMaterialKey {
    run: Entity,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PanelShapeMaterialKey {
    panel:             Entity,
    primitive_ordinal: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MaterialSlotKey {
    SdfSurface(SdfSurfaceMaterialKey),
    TextRun(TextRunMaterialKey),
    PanelShape(PanelShapeMaterialKey),
}

impl MaterialSlotKey {
    fn owner(self) -> SourceOwner {
        match self {
            Self::SdfSurface(key) => SourceOwner::Sdf(key.panel),
            Self::TextRun(key) => SourceOwner::Text(key.run),
            Self::PanelShape(key) => SourceOwner::PanelShape(key.panel),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MaterialSlotValues {
    source_marker: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LifetimeStrategy {
    Simple { retirement_frames: u64 },
    Generation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RetiredSlot {
    id:                MaterialSlotId,
    reusable_at_frame: u64,
}

#[derive(Clone, Debug)]
struct SlotEntry {
    key:        Option<MaterialSlotKey>,
    generation: MaterialSlotGeneration,
    values:     Option<MaterialSlotValues>,
}

impl Default for SlotEntry {
    fn default() -> Self {
        Self {
            key:        None,
            generation: MaterialSlotGeneration(0),
            values:     None,
        }
    }
}

#[derive(Debug, Resource)]
struct SharedMaterialTable {
    strategy:   LifetimeStrategy,
    rows:       Vec<SlotEntry>,
    key_slots:  HashMap<MaterialSlotKey, MaterialSlotId>,
    owner_keys: HashMap<SourceOwner, HashSet<MaterialSlotKey>>,
    retired:    VecDeque<RetiredSlot>,
}

impl SharedMaterialTable {
    fn new(strategy: LifetimeStrategy) -> Self {
        Self {
            strategy,
            rows: Vec::new(),
            key_slots: HashMap::new(),
            owner_keys: HashMap::new(),
            retired: VecDeque::new(),
        }
    }

    fn alloc(
        &mut self,
        key: MaterialSlotKey,
        values: MaterialSlotValues,
        frame: u64,
    ) -> SlotPointer {
        if let Some(id) = self.key_slots.get(&key).copied() {
            self.rows[id.index()].values = Some(values);
            return self.pointer_for(id);
        }

        let id = self
            .take_reusable_slot(frame)
            .unwrap_or_else(|| self.push_slot());
        let row = &mut self.rows[id.index()];
        if matches!(self.strategy, LifetimeStrategy::Generation) {
            row.generation = row.generation.next();
        }
        row.key = Some(key);
        row.values = Some(values);
        self.key_slots.insert(key, id);
        self.owner_keys.entry(key.owner()).or_default().insert(key);
        self.pointer_for(id)
    }

    fn free(&mut self, key: MaterialSlotKey, frame: u64) {
        let Some(id) = self.key_slots.remove(&key) else {
            return;
        };
        let row = &mut self.rows[id.index()];
        row.key = None;
        row.values = None;
        if let Some(keys) = self.owner_keys.get_mut(&key.owner()) {
            keys.remove(&key);
        }
        self.retired.push_back(RetiredSlot {
            id,
            reusable_at_frame: self.reusable_at_frame(frame),
        });
    }

    fn capacity(&self) -> usize { self.rows.len() }

    fn live_slot_count(&self) -> usize { self.key_slots.len() }

    fn finish_scope(
        &mut self,
        owner: SourceOwner,
        live_keys: &HashSet<MaterialSlotKey>,
        frame: u64,
    ) {
        let previous = self.owner_keys.get(&owner).cloned().unwrap_or_default();
        for key in previous.difference(live_keys) {
            self.free(*key, frame);
        }
    }

    fn free_owner(&mut self, owner: SourceOwner, frame: u64) {
        let previous = self.owner_keys.get(&owner).cloned().unwrap_or_default();
        for key in previous {
            self.free(key, frame);
        }
    }

    fn read(&self, pointer: SlotPointer) -> Option<MaterialSlotValues> {
        let row = self.rows.get(pointer.id().index())?;
        match pointer {
            SlotPointer::Id(_) => row.values,
            SlotPointer::Ref(reference) if row.generation == reference.generation => row.values,
            SlotPointer::Ref(_) => None,
        }
    }

    fn pointer_for(&self, id: MaterialSlotId) -> SlotPointer {
        match self.strategy {
            LifetimeStrategy::Simple { .. } => SlotPointer::Id(id),
            LifetimeStrategy::Generation => SlotPointer::Ref(MaterialSlotRef {
                id,
                generation: self.rows[id.index()].generation,
            }),
        }
    }

    fn push_slot(&mut self) -> MaterialSlotId {
        let id = MaterialSlotId(self.rows.len().to_u32());
        self.rows.push(SlotEntry::default());
        id
    }

    fn reusable_at_frame(&self, frame: u64) -> u64 {
        match self.strategy {
            LifetimeStrategy::Simple { retirement_frames } => frame + retirement_frames,
            LifetimeStrategy::Generation => frame,
        }
    }

    fn take_reusable_slot(&mut self, frame: u64) -> Option<MaterialSlotId> {
        let index = self
            .retired
            .iter()
            .position(|retired| retired.reusable_at_frame <= frame)?;
        self.retired.remove(index).map(|retired| retired.id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RemovedRecordPolicy {
    Drop,
    RetainStale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RetainedRecord {
    pointer: SlotPointer,
}

#[derive(Debug, Default)]
struct ProducerStore {
    records: HashMap<MaterialSlotKey, RetainedRecord>,
}

impl ProducerStore {
    fn upsert(
        &mut self,
        table: &mut SharedMaterialTable,
        key: MaterialSlotKey,
        values: MaterialSlotValues,
        frame: u64,
    ) -> RetainedRecord {
        let pointer = table.alloc(key, values, frame);
        let record = RetainedRecord { pointer };
        self.records.insert(key, record);
        record
    }

    fn remove(
        &mut self,
        table: &mut SharedMaterialTable,
        key: MaterialSlotKey,
        frame: u64,
        removed_record_policy: RemovedRecordPolicy,
    ) {
        table.free(key, frame);
        if removed_record_policy == RemovedRecordPolicy::Drop {
            self.records.remove(&key);
        }
    }

    fn stale_record(&self, key: MaterialSlotKey) -> Option<RetainedRecord> {
        self.records.get(&key).copied()
    }

    fn retain_live(&mut self, live_keys: &HashSet<MaterialSlotKey>) {
        self.records.retain(|key, _| live_keys.contains(key));
    }
}

#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ScheduledProbeSet {
    Producers,
    ApplyRequests,
    Rebind,
    Extract,
    Prepare,
}

#[derive(Clone, Copy, Debug)]
enum ScheduledProbeOrder {
    Correct,
    ExtractBeforeRebind,
}

#[derive(Clone, Copy, Debug, Resource)]
struct ProbeFrame(u64);

#[derive(Clone, Copy, Debug, Resource)]
struct ProbeEntities {
    panel: Entity,
    run:   Entity,
}

#[derive(Clone, Copy, Debug)]
struct SourceSpec {
    ordinal:       u32,
    source_marker: u32,
}

#[derive(Debug, Default, Resource)]
struct ScheduledSourceInputs {
    sdf_surfaces: Vec<SourceSpec>,
    text_runs:    Vec<SourceSpec>,
    panel_shapes: Vec<SourceSpec>,
}

#[derive(Debug, Default, Resource)]
struct SdfProducerStore(ProducerStore);

#[derive(Debug, Default, Resource)]
struct TextProducerStore(ProducerStore);

#[derive(Debug, Default, Resource)]
struct ShapeProducerStore(ProducerStore);

#[derive(Clone, Copy, Debug)]
struct MaterialRequest {
    key:    MaterialSlotKey,
    values: MaterialSlotValues,
}

#[derive(Debug, Default, Resource)]
struct SdfMaterialRequests {
    owner: SourceOwner,
    live:  Vec<MaterialRequest>,
}

#[derive(Debug, Default, Resource)]
struct TextMaterialRequests {
    owner: SourceOwner,
    live:  Vec<MaterialRequest>,
}

#[derive(Debug, Default, Resource)]
struct ShapeMaterialRequests {
    owner: SourceOwner,
    live:  Vec<MaterialRequest>,
}

#[derive(Debug, Default, Resource)]
struct RegisteredBatchMaterials {
    bound_table_capacity: usize,
    rebinds:              u32,
}

#[derive(Debug, Default, Resource)]
struct ExtractedProbeSnapshot {
    table_capacity:       usize,
    bound_table_capacity: usize,
    live_reads:           Vec<Option<MaterialSlotValues>>,
}

#[derive(Debug, Default, Resource)]
struct PreparedProbeSnapshot {
    binding_matches_table: bool,
    live_records_read:     bool,
}

#[derive(Clone, Debug, Default)]
struct ExtractedRecords {
    records: Vec<RetainedRecord>,
}

impl ExtractedRecords {
    fn read_all(&self, table: &SharedMaterialTable) -> Vec<Option<MaterialSlotValues>> {
        self.records
            .iter()
            .map(|record| table.read(record.pointer))
            .collect()
    }
}

#[derive(Component, Clone, Copy, Debug, Eq, PartialEq)]
#[relationship(relationship_target = ProbeRuns)]
struct ProbeRunOf(#[entities] Entity);

#[derive(Component, Debug, Default, Eq, PartialEq)]
#[relationship_target(relationship = ProbeRunOf)]
struct ProbeRuns(Vec<Entity>);

#[derive(Component, Clone, Copy, Debug, Eq, PartialEq)]
#[relationship(relationship_target = ProbeOwnerSlots)]
struct ProbeSlotEntityOf(#[entities] Entity);

#[derive(Component, Debug, Default, Eq, PartialEq)]
#[relationship_target(relationship = ProbeSlotEntityOf)]
struct ProbeOwnerSlots(Vec<Entity>);

fn sdf_key(panel: Entity, command_index: u32) -> MaterialSlotKey {
    MaterialSlotKey::SdfSurface(SdfSurfaceMaterialKey {
        panel,
        command_index,
    })
}

fn text_key(run: Entity) -> MaterialSlotKey { MaterialSlotKey::TextRun(TextRunMaterialKey { run }) }

fn panel_shape_key(panel: Entity, primitive_ordinal: u32) -> MaterialSlotKey {
    MaterialSlotKey::PanelShape(PanelShapeMaterialKey {
        panel,
        primitive_ordinal,
    })
}

fn values(source_marker: u32) -> MaterialSlotValues { MaterialSlotValues { source_marker } }

fn frame_u64(frame: usize) -> u64 { u64::from(frame.to_u32()) }

fn collect_sdf_requests(
    entities: Res<ProbeEntities>,
    inputs: Res<ScheduledSourceInputs>,
    mut requests: ResMut<SdfMaterialRequests>,
) {
    requests.owner = SourceOwner::Sdf(entities.panel);
    requests.live = inputs
        .sdf_surfaces
        .iter()
        .map(|source| MaterialRequest {
            key:    sdf_key(entities.panel, source.ordinal),
            values: values(source.source_marker),
        })
        .collect();
}

fn collect_text_requests(
    entities: Res<ProbeEntities>,
    inputs: Res<ScheduledSourceInputs>,
    mut requests: ResMut<TextMaterialRequests>,
) {
    requests.owner = SourceOwner::Text(entities.run);
    requests.live = inputs
        .text_runs
        .iter()
        .map(|source| MaterialRequest {
            key:    text_key(entities.run),
            values: values(source.source_marker),
        })
        .collect();
}

fn collect_shape_requests(
    entities: Res<ProbeEntities>,
    inputs: Res<ScheduledSourceInputs>,
    mut requests: ResMut<ShapeMaterialRequests>,
) {
    requests.owner = SourceOwner::PanelShape(entities.panel);
    requests.live = inputs
        .panel_shapes
        .iter()
        .map(|source| MaterialRequest {
            key:    panel_shape_key(entities.panel, source.ordinal),
            values: values(source.source_marker),
        })
        .collect();
}

fn apply_material_requests(
    frame: Res<ProbeFrame>,
    mut table: ResMut<SharedMaterialTable>,
    sdf_requests: Res<SdfMaterialRequests>,
    text_requests: Res<TextMaterialRequests>,
    shape_requests: Res<ShapeMaterialRequests>,
    mut sdf_store: ResMut<SdfProducerStore>,
    mut text_store: ResMut<TextProducerStore>,
    mut shape_store: ResMut<ShapeProducerStore>,
) {
    apply_producer_requests(
        &mut table,
        &mut sdf_store.0,
        sdf_requests.owner,
        &sdf_requests.live,
        frame.0,
    );
    apply_producer_requests(
        &mut table,
        &mut text_store.0,
        text_requests.owner,
        &text_requests.live,
        frame.0,
    );
    apply_producer_requests(
        &mut table,
        &mut shape_store.0,
        shape_requests.owner,
        &shape_requests.live,
        frame.0,
    );
}

fn apply_producer_requests(
    table: &mut SharedMaterialTable,
    store: &mut ProducerStore,
    owner: SourceOwner,
    requests: &[MaterialRequest],
    frame: u64,
) {
    let live_keys: HashSet<MaterialSlotKey> = requests.iter().map(|request| request.key).collect();
    for request in requests {
        store.upsert(table, request.key, request.values, frame);
    }
    table.finish_scope(owner, &live_keys, frame);
    store.retain_live(&live_keys);
}

fn rebind_registered_materials(
    table: Res<SharedMaterialTable>,
    mut registry: ResMut<RegisteredBatchMaterials>,
) {
    let capacity = table.capacity();
    if registry.bound_table_capacity != capacity {
        registry.bound_table_capacity = capacity;
        registry.rebinds += 1;
    }
}

fn extract_probe_snapshot(
    table: Res<SharedMaterialTable>,
    registry: Res<RegisteredBatchMaterials>,
    sdf_store: Res<SdfProducerStore>,
    text_store: Res<TextProducerStore>,
    shape_store: Res<ShapeProducerStore>,
    mut extracted: ResMut<ExtractedProbeSnapshot>,
) {
    let live_reads = sdf_store
        .0
        .records
        .values()
        .chain(text_store.0.records.values())
        .chain(shape_store.0.records.values())
        .map(|record| table.read(record.pointer))
        .collect();
    *extracted = ExtractedProbeSnapshot {
        table_capacity: table.capacity(),
        bound_table_capacity: registry.bound_table_capacity,
        live_reads,
    };
}

fn prepare_probe_snapshot(
    extracted: Res<ExtractedProbeSnapshot>,
    mut prepared: ResMut<PreparedProbeSnapshot>,
) {
    *prepared = PreparedProbeSnapshot {
        binding_matches_table: extracted.bound_table_capacity == extracted.table_capacity,
        live_records_read:     extracted.live_reads.iter().all(Option::is_some),
    };
}

fn scheduled_probe_app(strategy: LifetimeStrategy, order: ScheduledProbeOrder) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    let panel = app.world_mut().spawn_empty().id();
    let run = app.world_mut().spawn_empty().id();
    app.insert_resource(ProbeFrame(0));
    app.insert_resource(ProbeEntities { panel, run });
    app.insert_resource(ScheduledSourceInputs::default());
    app.insert_resource(SharedMaterialTable::new(strategy));
    app.init_resource::<SdfProducerStore>();
    app.init_resource::<TextProducerStore>();
    app.init_resource::<ShapeProducerStore>();
    app.init_resource::<SdfMaterialRequests>();
    app.init_resource::<TextMaterialRequests>();
    app.init_resource::<ShapeMaterialRequests>();
    app.init_resource::<RegisteredBatchMaterials>();
    app.init_resource::<ExtractedProbeSnapshot>();
    app.init_resource::<PreparedProbeSnapshot>();
    configure_scheduled_probe(&mut app, order);
    app
}

fn configure_scheduled_probe(app: &mut App, order: ScheduledProbeOrder) {
    match order {
        ScheduledProbeOrder::Correct => {
            app.configure_sets(
                Update,
                (
                    ScheduledProbeSet::Producers,
                    ScheduledProbeSet::ApplyRequests,
                    ScheduledProbeSet::Rebind,
                    ScheduledProbeSet::Extract,
                    ScheduledProbeSet::Prepare,
                )
                    .chain(),
            );
        },
        ScheduledProbeOrder::ExtractBeforeRebind => {
            app.configure_sets(
                Update,
                (
                    ScheduledProbeSet::Producers,
                    ScheduledProbeSet::ApplyRequests,
                    ScheduledProbeSet::Extract,
                    ScheduledProbeSet::Rebind,
                    ScheduledProbeSet::Prepare,
                )
                    .chain(),
            );
        },
    }
    app.add_systems(
        Update,
        (
            collect_sdf_requests,
            collect_text_requests,
            collect_shape_requests,
        )
            .in_set(ScheduledProbeSet::Producers),
    );
    app.add_systems(
        Update,
        apply_material_requests.in_set(ScheduledProbeSet::ApplyRequests),
    );
    app.add_systems(
        Update,
        rebind_registered_materials.in_set(ScheduledProbeSet::Rebind),
    );
    app.add_systems(
        Update,
        extract_probe_snapshot.in_set(ScheduledProbeSet::Extract),
    );
    app.add_systems(
        Update,
        prepare_probe_snapshot.in_set(ScheduledProbeSet::Prepare),
    );
}

fn set_scheduled_inputs(
    app: &mut App,
    frame: u64,
    sdf_surfaces: Vec<SourceSpec>,
    text_runs: Vec<SourceSpec>,
    panel_shapes: Vec<SourceSpec>,
) {
    app.world_mut().resource_mut::<ProbeFrame>().0 = frame;
    *app.world_mut().resource_mut::<ScheduledSourceInputs>() = ScheduledSourceInputs {
        sdf_surfaces,
        text_runs,
        panel_shapes,
    };
}

fn stale_record_values(
    table: &SharedMaterialTable,
    producer: &ProducerStore,
    key: MaterialSlotKey,
) -> Option<MaterialSlotValues> {
    producer
        .stale_record(key)
        .and_then(|record| table.read(record.pointer))
}

fn extracted_records(store: &ProducerStore) -> ExtractedRecords {
    ExtractedRecords {
        records: store.records.values().copied().collect(),
    }
}

fn stress_keys(panel: Entity, frame: usize) -> Vec<MaterialSlotKey> {
    let frame_offset = frame * STRESS_SLOT_COUNT;
    (0..STRESS_SLOT_COUNT)
        .map(|index| panel_shape_key(panel, (frame_offset + index).to_u32()))
        .collect()
}

fn replace_live_keys(
    table: &mut SharedMaterialTable,
    producer: &mut ProducerStore,
    frame: usize,
    previous_keys: &[MaterialSlotKey],
    next_keys: &[MaterialSlotKey],
) {
    for key in previous_keys {
        producer.remove(table, *key, frame_u64(frame), RemovedRecordPolicy::Drop);
    }
    for (index, key) in next_keys.iter().enumerate() {
        producer.upsert(table, *key, values(index.to_u32()), frame_u64(frame));
    }
}

fn relationship_run_entities(world: &World, panel: Entity) -> HashSet<Entity> {
    world
        .get::<ProbeRuns>(panel)
        .map_or_else(HashSet::new, |runs| runs.0.iter().copied().collect())
}

fn free_text_owners_not_in_relationship(
    table: &mut SharedMaterialTable,
    live_runs: &HashSet<Entity>,
    frame: u64,
) {
    let stale_owners: Vec<SourceOwner> = table
        .owner_keys
        .keys()
        .copied()
        .filter(|owner| match owner {
            SourceOwner::Text(run) => !live_runs.contains(run),
            SourceOwner::Sdf(_) | SourceOwner::PanelShape(_) => false,
        })
        .collect();
    for owner in stale_owners {
        table.free_owner(owner, frame);
    }
}

#[test]
fn immediate_reuse_with_bare_slot_id_exposes_reused_row_to_stale_record() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let source_a = sdf_key(panel, 10);
    let source_b = sdf_key(panel, 11);
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Simple {
        retirement_frames: 0,
    });
    let mut producer = ProducerStore::default();

    producer.upsert(&mut table, source_a, values(100), 1);
    producer.remove(&mut table, source_a, 1, RemovedRecordPolicy::RetainStale);
    producer.upsert(&mut table, source_b, values(200), 1);

    assert_eq!(
        stale_record_values(&table, &producer, source_a),
        Some(values(200)),
        "bare ids with immediate reuse let a stale record read the new source row",
    );
}

#[test]
fn delayed_reuse_keeps_bare_slot_id_from_reading_new_source() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let source_a = sdf_key(panel, 10);
    let source_b = sdf_key(panel, 11);
    let source_c = sdf_key(panel, 12);
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Simple {
        retirement_frames: 2,
    });
    let mut producer = ProducerStore::default();

    let first = producer.upsert(&mut table, source_a, values(100), 1);
    producer.remove(&mut table, source_a, 1, RemovedRecordPolicy::RetainStale);
    let same_frame = producer.upsert(&mut table, source_b, values(200), 1);

    assert_ne!(
        first.pointer.id(),
        same_frame.pointer.id(),
        "same-frame allocation must not reuse a retired bare id",
    );
    assert_eq!(
        stale_record_values(&table, &producer, source_a),
        None,
        "the stale bare-id record sees no live row while the slot is retired",
    );

    let later = producer.upsert(&mut table, source_c, values(300), 3);
    assert_eq!(
        first.pointer.id(),
        later.pointer.id(),
        "the bare id becomes reusable only after the retirement boundary",
    );
}

#[test]
fn generation_ref_rejects_stale_record_after_same_frame_reuse() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let source_a = sdf_key(panel, 10);
    let source_b = sdf_key(panel, 11);
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Generation);
    let mut producer = ProducerStore::default();

    let first = producer.upsert(&mut table, source_a, values(100), 1);
    producer.remove(&mut table, source_a, 1, RemovedRecordPolicy::RetainStale);
    let same_frame = producer.upsert(&mut table, source_b, values(200), 1);

    assert_eq!(
        first.pointer.id(),
        same_frame.pointer.id(),
        "generation strategy may reuse a physical row immediately",
    );
    assert_eq!(
        stale_record_values(&table, &producer, source_a),
        None,
        "generation mismatch rejects the stale record",
    );
    assert_eq!(
        table.read(same_frame.pointer),
        Some(values(200)),
        "the current record still reads the reused row",
    );
}

#[test]
fn delayed_reuse_handles_independent_producer_stores_without_record_rewrite() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let run = world.spawn_empty().id();
    let text = text_key(run);
    let shape = panel_shape_key(panel, 0);
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Simple {
        retirement_frames: 2,
    });
    let mut text_store = ProducerStore::default();
    let mut shape_store = ProducerStore::default();

    let text_record = text_store.upsert(&mut table, text, values(10), 1);
    text_store.remove(&mut table, text, 1, RemovedRecordPolicy::RetainStale);
    let shape_record = shape_store.upsert(&mut table, shape, values(20), 1);

    assert_ne!(
        text_record.pointer.id(),
        shape_record.pointer.id(),
        "one producer's retired bare id must not be reused by another producer in the same frame",
    );
    assert_eq!(
        stale_record_values(&table, &text_store, text),
        None,
        "a lagging producer record cannot read another producer's new material",
    );
}

#[test]
fn table_owned_scope_frees_removed_owner_keys_without_touching_other_owners() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let run = world.spawn_empty().id();
    let sdf = sdf_key(panel, 10);
    let first_shape = panel_shape_key(panel, 0);
    let second_shape = panel_shape_key(panel, 1);
    let text = text_key(run);
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Simple {
        retirement_frames: 1,
    });

    let sdf_record = table.alloc(sdf, values(1), 1);
    let first_shape_record = table.alloc(first_shape, values(2), 1);
    let second_shape_record = table.alloc(second_shape, values(3), 1);
    let text_record = table.alloc(text, values(4), 1);

    let mut live_shape_keys = HashSet::new();
    live_shape_keys.insert(second_shape);
    table.finish_scope(SourceOwner::PanelShape(panel), &live_shape_keys, 2);

    assert_eq!(
        table.read(first_shape_record),
        None,
        "scope cleanup frees removed panel-shape slots",
    );
    assert_eq!(
        table.read(second_shape_record),
        Some(values(3)),
        "scope cleanup preserves live panel-shape slots",
    );
    assert_eq!(
        table.read(sdf_record),
        Some(values(1)),
        "scope cleanup for panel shapes does not touch SDF slots",
    );
    assert_eq!(
        table.read(text_record),
        Some(values(4)),
        "scope cleanup for panel shapes does not touch text slots",
    );
}

#[test]
fn scheduled_rebind_after_allocation_keeps_extraction_consistent_on_growth() {
    let mut app = scheduled_probe_app(
        LifetimeStrategy::Simple {
            retirement_frames: 2,
        },
        ScheduledProbeOrder::Correct,
    );

    set_scheduled_inputs(
        &mut app,
        1,
        vec![SourceSpec {
            ordinal:       0,
            source_marker: 10,
        }],
        Vec::new(),
        Vec::new(),
    );
    app.update();
    let first_capacity = app
        .world()
        .resource::<ExtractedProbeSnapshot>()
        .table_capacity;

    set_scheduled_inputs(
        &mut app,
        2,
        vec![SourceSpec {
            ordinal:       0,
            source_marker: 11,
        }],
        vec![SourceSpec {
            ordinal:       0,
            source_marker: 20,
        }],
        vec![
            SourceSpec {
                ordinal:       0,
                source_marker: 30,
            },
            SourceSpec {
                ordinal:       1,
                source_marker: 31,
            },
        ],
    );
    app.update();

    let extracted = app.world().resource::<ExtractedProbeSnapshot>();
    let prepared = app.world().resource::<PreparedProbeSnapshot>();
    assert!(
        extracted.table_capacity > first_capacity,
        "the second frame grows the material table",
    );
    assert_eq!(
        extracted.bound_table_capacity, extracted.table_capacity,
        "rebind runs after allocation and before extraction",
    );
    assert!(
        prepared.binding_matches_table && prepared.live_records_read,
        "prepare observes a rebound material and readable live records",
    );
}

#[test]
fn scheduled_extraction_before_rebind_observes_stale_material_binding_on_growth() {
    let mut app = scheduled_probe_app(
        LifetimeStrategy::Simple {
            retirement_frames: 2,
        },
        ScheduledProbeOrder::ExtractBeforeRebind,
    );

    set_scheduled_inputs(
        &mut app,
        1,
        vec![SourceSpec {
            ordinal:       0,
            source_marker: 10,
        }],
        Vec::new(),
        Vec::new(),
    );
    app.update();

    set_scheduled_inputs(
        &mut app,
        2,
        vec![SourceSpec {
            ordinal:       0,
            source_marker: 11,
        }],
        vec![SourceSpec {
            ordinal:       0,
            source_marker: 20,
        }],
        vec![SourceSpec {
            ordinal:       0,
            source_marker: 30,
        }],
    );
    app.update();

    let extracted = app.world().resource::<ExtractedProbeSnapshot>();
    let prepared = app.world().resource::<PreparedProbeSnapshot>();
    assert!(
        extracted.bound_table_capacity < extracted.table_capacity,
        "extraction before rebind captures the old material-table binding",
    );
    assert!(
        !prepared.binding_matches_table && prepared.live_records_read,
        "the failure is specifically stale binding, not unreadable live records",
    );
}

#[test]
fn scheduled_material_edit_updates_row_without_rebind_or_pointer_change() {
    let mut app = scheduled_probe_app(
        LifetimeStrategy::Simple {
            retirement_frames: 2,
        },
        ScheduledProbeOrder::Correct,
    );

    set_scheduled_inputs(
        &mut app,
        1,
        Vec::new(),
        vec![SourceSpec {
            ordinal:       0,
            source_marker: 20,
        }],
        Vec::new(),
    );
    app.update();
    let entities = *app.world().resource::<ProbeEntities>();
    let text = text_key(entities.run);
    let first_record = app
        .world()
        .resource::<TextProducerStore>()
        .0
        .stale_record(text);
    let first_rebinds = app.world().resource::<RegisteredBatchMaterials>().rebinds;

    set_scheduled_inputs(
        &mut app,
        2,
        Vec::new(),
        vec![SourceSpec {
            ordinal:       0,
            source_marker: 21,
        }],
        Vec::new(),
    );
    app.update();

    let table = app.world().resource::<SharedMaterialTable>();
    let second_record = app
        .world()
        .resource::<TextProducerStore>()
        .0
        .stale_record(text);
    let second_rebinds = app.world().resource::<RegisteredBatchMaterials>().rebinds;

    assert_eq!(
        first_record, second_record,
        "material-only edits keep the existing record pointer",
    );
    assert_eq!(
        second_record.and_then(|record| table.read(record.pointer)),
        Some(values(21)),
        "the table row still receives the new material values",
    );
    assert_eq!(
        first_rebinds, second_rebinds,
        "same-capacity material edits do not rebind registered materials",
    );
}

#[test]
fn one_frame_retirement_allows_previous_extraction_snapshot_to_read_reused_row() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let source_a = sdf_key(panel, 10);
    let source_b = sdf_key(panel, 11);
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Simple {
        retirement_frames: 1,
    });
    let mut producer = ProducerStore::default();

    producer.upsert(&mut table, source_a, values(100), 1);
    let previous_extraction = extracted_records(&producer);
    producer.remove(&mut table, source_a, 1, RemovedRecordPolicy::Drop);
    producer.upsert(&mut table, source_b, values(200), 2);

    assert_eq!(
        previous_extraction.read_all(&table),
        vec![Some(values(200))],
        "a one-frame delay is unsafe if a previous extraction snapshot can survive into the next frame",
    );
}

#[test]
fn two_frame_retirement_blocks_previous_extraction_snapshot_from_reused_row() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let source_a = sdf_key(panel, 10);
    let source_b = sdf_key(panel, 11);
    let source_c = sdf_key(panel, 12);
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Simple {
        retirement_frames: 2,
    });
    let mut producer = ProducerStore::default();

    let first = producer.upsert(&mut table, source_a, values(100), 1);
    let previous_extraction = extracted_records(&producer);
    producer.remove(&mut table, source_a, 1, RemovedRecordPolicy::Drop);
    let next_frame = producer.upsert(&mut table, source_b, values(200), 2);

    assert_ne!(
        first.pointer.id(),
        next_frame.pointer.id(),
        "a two-frame delay prevents next-frame reuse",
    );
    assert_eq!(
        previous_extraction.read_all(&table),
        vec![None],
        "the previous extraction snapshot cannot read a new source row",
    );

    let later = producer.upsert(&mut table, source_c, values(300), 3);
    assert_eq!(
        first.pointer.id(),
        later.pointer.id(),
        "the slot is reusable once the previous extraction snapshot is beyond the modeled lag",
    );
}

#[test]
fn generation_ref_survives_record_buffer_lag_without_retirement() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let source_a = sdf_key(panel, 10);
    let source_b = sdf_key(panel, 11);
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Generation);
    let mut producer = ProducerStore::default();

    let first = producer.upsert(&mut table, source_a, values(100), 1);
    let previous_extraction = extracted_records(&producer);
    producer.remove(&mut table, source_a, 1, RemovedRecordPolicy::Drop);
    let next_frame = producer.upsert(&mut table, source_b, values(200), 2);

    assert_eq!(
        first.pointer.id(),
        next_frame.pointer.id(),
        "generation refs allow immediate physical row reuse",
    );
    assert_eq!(
        previous_extraction.read_all(&table),
        vec![None],
        "the generation mismatch protects lagged records",
    );
    assert_eq!(
        table.read(next_frame.pointer),
        Some(values(200)),
        "current records still read the reused row",
    );
}

#[test]
fn delayed_reuse_capacity_stabilizes_under_full_slot_churn() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Simple {
        retirement_frames: TWO_FRAME_RETIREMENT_U64,
    });
    let mut producer = ProducerStore::default();
    let mut live_keys = Vec::new();
    let expected_capacity = STRESS_SLOT_COUNT * (TWO_FRAME_RETIREMENT + 1);

    for frame in 1..=STRESS_FRAME_COUNT {
        let next_keys = stress_keys(panel, frame);
        replace_live_keys(&mut table, &mut producer, frame, &live_keys, &next_keys);
        live_keys = next_keys;
    }

    assert_eq!(
        table.live_slot_count(),
        STRESS_SLOT_COUNT,
        "full churn keeps only the current frame's slots live",
    );
    assert_eq!(
        table.capacity(),
        expected_capacity,
        "capacity stabilizes at live slots plus the two-frame retirement window",
    );
}

#[test]
fn generation_capacity_stays_at_live_count_under_full_slot_churn() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Generation);
    let mut producer = ProducerStore::default();
    let mut live_keys = Vec::new();

    for frame in 1..=STRESS_FRAME_COUNT {
        let next_keys = stress_keys(panel, frame);
        replace_live_keys(&mut table, &mut producer, frame, &live_keys, &next_keys);
        live_keys = next_keys;
    }

    assert_eq!(
        table.live_slot_count(),
        STRESS_SLOT_COUNT,
        "full churn keeps only the current frame's slots live",
    );
    assert_eq!(
        table.capacity(),
        STRESS_SLOT_COUNT,
        "generations allow immediate slot reuse under churn",
    );
}

#[test]
fn material_only_stress_does_not_grow_delayed_reuse_table() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let keys = stress_keys(panel, 1);
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Simple {
        retirement_frames: TWO_FRAME_RETIREMENT_U64,
    });
    let mut producer = ProducerStore::default();

    for frame in 1..=STRESS_FRAME_COUNT {
        for (index, key) in keys.iter().enumerate() {
            producer.upsert(
                &mut table,
                *key,
                values((frame * STRESS_SLOT_COUNT + index).to_u32()),
                frame_u64(frame),
            );
        }
    }

    assert_eq!(
        table.live_slot_count(),
        STRESS_SLOT_COUNT,
        "material-only edits keep the same source-owned slots live",
    );
    assert_eq!(
        table.capacity(),
        STRESS_SLOT_COUNT,
        "material-only edits do not allocate replacement slots",
    );
}

#[test]
fn relationship_run_removal_guides_text_slot_cleanup_without_slot_entities() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let first_run = world.spawn(ProbeRunOf(panel)).id();
    let second_run = world.spawn(ProbeRunOf(panel)).id();
    let first_text = text_key(first_run);
    let second_text = text_key(second_run);
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Simple {
        retirement_frames: TWO_FRAME_RETIREMENT_U64,
    });

    let first_record = table.alloc(first_text, values(10), 1);
    let second_record = table.alloc(second_text, values(20), 1);
    world.entity_mut(first_run).despawn();
    let live_runs = relationship_run_entities(&world, panel);
    free_text_owners_not_in_relationship(&mut table, &live_runs, 2);

    assert_eq!(
        table.read(first_record),
        None,
        "relationship membership identifies the removed text source",
    );
    assert_eq!(
        table.read(second_record),
        Some(values(20)),
        "slot cleanup preserves the remaining run's material",
    );
    assert_eq!(
        table.live_slot_count(),
        1,
        "only the still-related text run remains live",
    );
}

#[test]
fn text_run_reuse_updates_values_without_relationship_or_slot_churn() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let run = world.spawn(ProbeRunOf(panel)).id();
    let text = text_key(run);
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Simple {
        retirement_frames: TWO_FRAME_RETIREMENT_U64,
    });
    let initial_runs = relationship_run_entities(&world, panel);

    let first_record = table.alloc(text, values(10), 1);
    let second_record = table.alloc(text, values(20), 2);
    let final_runs = relationship_run_entities(&world, panel);

    assert_eq!(
        first_record, second_record,
        "reusing a text source keeps the same slot pointer",
    );
    assert_eq!(
        table.read(second_record),
        Some(values(20)),
        "text source reuse updates the row value in place",
    );
    assert_eq!(
        initial_runs, final_runs,
        "material updates do not mutate the source relationship set",
    );
    assert_eq!(table.capacity(), 1, "text reuse does not allocate slots");
}

#[test]
fn panel_shape_scope_cleanup_frees_only_removed_primitives() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Simple {
        retirement_frames: TWO_FRAME_RETIREMENT_U64,
    });
    let keys: Vec<MaterialSlotKey> = (0..8)
        .map(|primitive_ordinal| panel_shape_key(panel, primitive_ordinal))
        .collect();
    let records: Vec<SlotPointer> = keys
        .iter()
        .enumerate()
        .map(|(index, key)| table.alloc(*key, values(index.to_u32()), 1))
        .collect();
    let live_keys: HashSet<MaterialSlotKey> = keys
        .iter()
        .enumerate()
        .filter_map(|(index, key)| (index % 2 == 0).then_some(*key))
        .collect();

    table.finish_scope(SourceOwner::PanelShape(panel), &live_keys, 2);

    for (index, record) in records.iter().copied().enumerate() {
        let expected = (index % 2 == 0).then_some(values(index.to_u32()));
        assert_eq!(
            table.read(record),
            expected,
            "shape primitive {index} cleanup should match the live-key set",
        );
    }
    assert_eq!(
        table.live_slot_count(),
        live_keys.len(),
        "scope cleanup retains only live shape primitive slots",
    );
}

#[test]
fn panel_owner_cleanup_frees_shape_and_sdf_slots_without_text_slots() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let run = world.spawn_empty().id();
    let sdf = sdf_key(panel, 10);
    let first_shape = panel_shape_key(panel, 0);
    let second_shape = panel_shape_key(panel, 1);
    let text = text_key(run);
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Simple {
        retirement_frames: TWO_FRAME_RETIREMENT_U64,
    });

    let sdf_record = table.alloc(sdf, values(10), 1);
    let first_shape_record = table.alloc(first_shape, values(20), 1);
    let second_shape_record = table.alloc(second_shape, values(21), 1);
    let text_record = table.alloc(text, values(30), 1);
    table.free_owner(SourceOwner::Sdf(panel), 2);
    table.free_owner(SourceOwner::PanelShape(panel), 2);

    assert_eq!(
        table.read(sdf_record),
        None,
        "panel cleanup releases SDF surface slots",
    );
    assert_eq!(
        table.read(first_shape_record),
        None,
        "panel cleanup releases first shape slot",
    );
    assert_eq!(
        table.read(second_shape_record),
        None,
        "panel cleanup releases second shape slot",
    );
    assert_eq!(
        table.read(text_record),
        Some(values(30)),
        "panel cleanup does not touch unrelated text source slots",
    );
}

#[test]
fn sdf_scope_cleanup_does_not_need_surface_entities() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let first_sdf = sdf_key(panel, 0);
    let second_sdf = sdf_key(panel, 1);
    let mut table = SharedMaterialTable::new(LifetimeStrategy::Simple {
        retirement_frames: TWO_FRAME_RETIREMENT_U64,
    });

    let first_record = table.alloc(first_sdf, values(10), 1);
    let second_record = table.alloc(second_sdf, values(20), 1);
    let live_keys = HashSet::from([second_sdf]);
    table.finish_scope(SourceOwner::Sdf(panel), &live_keys, 2);

    assert_eq!(
        table.read(first_record),
        None,
        "SDF surface cleanup uses source keys, not per-surface entities",
    );
    assert_eq!(
        table.read(second_record),
        Some(values(20)),
        "live SDF surface key is preserved",
    );
}

#[test]
fn slot_entity_model_creates_topology_churn_entities() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let mut previous_slots: Vec<Entity> = Vec::new();
    let mut created_slot_entities = 0usize;

    for _frame in 0..STRESS_FRAME_COUNT {
        for slot in std::mem::take(&mut previous_slots) {
            world.entity_mut(slot).despawn();
        }
        previous_slots = (0..STRESS_SLOT_COUNT)
            .map(|_| {
                created_slot_entities += 1;
                world.spawn(ProbeSlotEntityOf(panel)).id()
            })
            .collect();
    }

    let owner_slots = world
        .get::<ProbeOwnerSlots>(panel)
        .map_or_else(Vec::new, |slots| slots.0.clone());
    assert_eq!(
        owner_slots.len(),
        STRESS_SLOT_COUNT,
        "relationship target tracks only the current slot entities",
    );
    assert_eq!(
        created_slot_entities,
        STRESS_SLOT_COUNT * STRESS_FRAME_COUNT,
        "slot entities create entity churn under full topology churn unless they reimplement table-style reuse",
    );
}

#[test]
fn bevy_relationship_tracks_entity_backed_runs_without_slot_entities() {
    let mut world = World::new();
    let panel = world.spawn_empty().id();
    let first_run = world.spawn(ProbeRunOf(panel)).id();
    let second_run = world.spawn(ProbeRunOf(panel)).id();

    let run_entities = world
        .get::<ProbeRuns>(panel)
        .map_or_else(Vec::new, |runs| runs.0.clone());
    assert_eq!(run_entities.len(), 2, "both runs are indexed by the panel");
    assert!(
        run_entities.contains(&first_run) && run_entities.contains(&second_run),
        "relationship target stores the run entities",
    );

    world.entity_mut(first_run).despawn();
    let run_entities = world
        .get::<ProbeRuns>(panel)
        .map_or_else(Vec::new, |runs| runs.0.clone());
    assert_eq!(
        run_entities.as_slice(),
        &[second_run],
        "despawning a source entity removes it from the relationship target",
    );
}
