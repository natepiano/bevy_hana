//! Shared batch-store primitives and render-family taxonomy.
//!
//! A batch member is one element's draw contribution. A batch groups members by
//! a GPU-compatibility key. A store routes members to batches and keeps the
//! member-to-batch index.
//!
//! Membership units differ by family:
//! - SDF: per record; an element contributes one record, or two when a clipped border splits fill
//!   and border.
//! - Image: per record, with one record per image.
//! - Text: per run; each text run owns its glyph quads.
//! - `ShapeBatch`: per `PanelShapeRenderKey` group. Groups are element-local, but `ShapeBatchStore`
//!   receives them per panel because `ResolvedPanelShape` records are re-derived from the panel's
//!   resolved command stream each route pass.
//!
//! [`BatchEntry`] and [`take_empty_batches`] apply to all four families.
//! [`BatchStore`] owns member routing for SDF batches, image batches, and
//! `TextRunBatchStore` text-run batches directly. `ShapeBatch` routing uses
//! [`BatchStore`] behind `ShapeBatchStore` because its source records
//! arrive panel-by-panel from the resolved command stream.
//!
//! `SdfMemberFamily` and `ImageMemberFamily` are the only [`MemberFamily`]
//! implementers; their `Store` associated types are `SdfBatchStore` and
//! `ImageBatchStore`. Those families update per-member world transforms after
//! `TransformSystems::Propagate` and then recompute batch bounds from retained
//! records. Text also writes post-propagation transforms, but
//! `write_batch_run_transforms` reaches `TextRunBatchStore` through `GlyphCache`,
//! so it stays concrete. `ShapeBatchStore` applies the panel transform while
//! building its `PathRenderRecord` runs.

use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;

use bevy::camera::primitives::Aabb;
use bevy::ecs::component::Mutable;
use bevy::math::Vec3A;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Mat4;
use bevy::prelude::Query;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::With;

use super::Dirty;
use crate::panel::DiegeticPanel;

/// Implemented by `SdfBatch`, `ImageBatch`, `TextRunBatch`, and `ShapeBatch`.
pub(super) trait BatchEntry {
    /// True when no members remain.
    fn is_empty(&self) -> bool;

    /// The spawned batch entity, if reconciled.
    fn entity(&self) -> Option<Entity>;
}

/// A batch's membership surface.
///
/// The store never inspects [`Self::Payload`]. Concrete batch types own payload
/// semantics such as GPU records, dirty flags, transform carry-over, and
/// equality checks.
pub(super) trait Batch: BatchEntry + Default {
    /// Identity for one retained member inside a batch family.
    type MemberKey: Copy + Eq + Hash;
    /// Payload routed into the concrete batch.
    type Payload;

    /// Adds a member not currently present in this batch.
    fn insert(&mut self, member: Self::MemberKey, payload: Self::Payload);

    /// Updates a member already present in this batch.
    fn update(&mut self, member: Self::MemberKey, payload: Self::Payload);

    /// Removes a member from this batch.
    fn remove(&mut self, member: Self::MemberKey);
}

/// One retained SDF or image member whose world transform is panel-relative.
pub(super) trait MemberRecord {
    /// Panel entity that owns this member.
    fn panel(&self) -> Entity;

    /// Current world transform stored on this member.
    fn transform(&self) -> Mat4;

    /// Rewrites this member's world transform from its panel transform.
    fn update_world_transform(&mut self, panel_transform: &GlobalTransform);
}

/// Batch internals needed by the shared post-propagation systems.
pub(super) trait MemberBatch: Batch {
    /// Retained CPU record type for this member family.
    type Record: MemberRecord;

    /// Mutable retained records.
    fn records_mut(&mut self) -> &mut [Self::Record];

    /// Dirty flag for record-buffer upload.
    fn record_upload_mut(&mut self) -> &mut Dirty;

    /// Dirty flag for bounds recomputation.
    fn bounds_update(&self) -> Dirty;

    /// Mutable dirty flag for bounds recomputation.
    fn bounds_update_mut(&mut self) -> &mut Dirty;

    /// World-space union of this batch's retained records.
    fn world_bounds(&self) -> Option<(Vec3, Vec3)>;
}

/// Store, batch, marker, and store-access hooks for a retained-member family.
pub(super) trait MemberFamily: 'static {
    /// Batch-store key for GPU-compatible batches.
    type Key: Clone + Eq + Hash;
    /// Concrete retained-member batch.
    type Batch: MemberBatch;
    /// Resource wrapper that owns the generic `BatchStore`.
    type Store: Resource<Mutability = Mutable>;
    /// Marker component on spawned batch entities.
    type Marker: Component;

    /// Mutable access to the wrapped `BatchStore`.
    fn store_mut(store: &mut Self::Store) -> &mut BatchStore<Self::Key, Self::Batch>;
}

/// Updates each retained member's world transform from its owning panel.
pub(super) fn update_batch_world_transforms<F: MemberFamily>(
    mut store: ResMut<F::Store>,
    panel_transforms: Query<&GlobalTransform, With<DiegeticPanel>>,
) {
    for (_, batch) in F::store_mut(&mut *store).batches_mut() {
        let mut transform_update = Dirty::No;
        for record in batch.records_mut() {
            let Ok(panel_transform) = panel_transforms.get(record.panel()) else {
                continue;
            };
            let previous = record.transform();
            record.update_world_transform(panel_transform);
            if record.transform() != previous {
                transform_update.mark();
            }
        }
        if transform_update.is_set() {
            batch.record_upload_mut().mark();
            batch.bounds_update_mut().mark();
        }
    }
}

/// Updates spawned batch entity placement and local `Aabb` from record bounds.
pub(super) fn update_batch_bounds<F: MemberFamily>(
    mut store: ResMut<F::Store>,
    mut batch_entities: Query<(&mut Transform, &mut GlobalTransform, &mut Aabb), With<F::Marker>>,
) {
    for (_, batch) in F::store_mut(&mut *store).batches_mut() {
        if !batch.bounds_update().is_set() {
            continue;
        }
        let Some(entity) = batch.entity() else {
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
        batch.bounds_update_mut().clear();
    }
}

/// Routes members into batches keyed by render compatibility.
#[derive(Debug)]
pub(super) struct BatchStore<K: Clone + Eq + Hash, B: Batch> {
    batches:      HashMap<K, B>,
    member_index: HashMap<B::MemberKey, K>,
}

impl<K: Clone + Eq + Hash, B: Batch> Default for BatchStore<K, B> {
    fn default() -> Self {
        Self {
            batches:      HashMap::new(),
            member_index: HashMap::new(),
        }
    }
}

impl<K: Clone + Eq + Hash, B: Batch> BatchStore<K, B> {
    /// Inserts, updates, or moves one retained member.
    pub(super) fn upsert(&mut self, key: K, member: B::MemberKey, payload: B::Payload) {
        if let Some(current) = self.member_index.get(&member) {
            if *current == key {
                if let Some(batch) = self.batches.get_mut(&key) {
                    batch.update(member, payload);
                }
                return;
            }
            let previous = current.clone();
            if let Some(batch) = self.batches.get_mut(&previous) {
                batch.remove(member);
            }
            self.member_index.remove(&member);
        }
        self.batches
            .entry(key.clone())
            .or_default()
            .insert(member, payload);
        self.member_index.insert(member, key);
    }

    /// Removes one retained member from its current batch.
    pub(super) fn remove(&mut self, member: B::MemberKey) {
        let Some(key) = self.member_index.remove(&member) else {
            return;
        };
        if let Some(batch) = self.batches.get_mut(&key) {
            batch.remove(member);
        }
    }

    /// Removes every retained member absent from the current append pass.
    pub(super) fn retain(&mut self, active: &HashSet<B::MemberKey>) {
        let stale: Vec<B::MemberKey> = self
            .member_index
            .keys()
            .copied()
            .filter(|member| !active.contains(member))
            .collect();
        for member in stale {
            self.remove(member);
        }
    }

    /// Whether a member is currently routed to any batch.
    #[must_use]
    pub(super) fn contains(&self, member: B::MemberKey) -> bool {
        self.member_index.contains_key(&member)
    }

    /// Current batch key for a routed member.
    #[must_use]
    pub(super) fn key_for(&self, member: B::MemberKey) -> Option<&K> {
        self.member_index.get(&member)
    }

    /// Mutable batch containing a routed member.
    pub(super) fn member_batch_mut(&mut self, member: B::MemberKey) -> Option<&mut B> {
        let key = self.member_index.get(&member)?;
        self.batches.get_mut(key)
    }

    /// One batch by key.
    #[must_use]
    pub(super) fn get(&self, key: &K) -> Option<&B> { self.batches.get(key) }

    /// One batch by key, mutable.
    pub(super) fn get_mut(&mut self, key: &K) -> Option<&mut B> { self.batches.get_mut(key) }

    /// All batches.
    pub(super) fn batches(&self) -> impl Iterator<Item = (&K, &B)> { self.batches.iter() }

    /// All batches, mutable.
    pub(super) fn batches_mut(&mut self) -> impl Iterator<Item = (&K, &mut B)> {
        self.batches.iter_mut()
    }

    /// Drops empty batch entries, returning their entities for despawn.
    pub(super) fn take_empty_batches(&mut self) -> Vec<Entity> {
        take_empty_batches(&mut self.batches)
    }
}

/// Drops empty batch entries, returning their entities for despawn.
fn take_empty_batches<K: Clone + Eq + Hash, B: BatchEntry>(
    batches: &mut HashMap<K, B>,
) -> Vec<Entity> {
    let empty: Vec<K> = batches
        .iter()
        .filter(|(_, batch)| batch.is_empty())
        .map(|(key, _)| key.clone())
        .collect();
    let mut entities = Vec::new();
    for key in empty {
        if let Some(batch) = batches.remove(&key)
            && let Some(entity) = batch.entity()
        {
            entities.push(entity);
        }
    }
    entities
}
