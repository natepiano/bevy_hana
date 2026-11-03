//! Anchor relationship resolver.

use std::hash::Hash;

use bevy_ecs::change_detection::DetectChanges;
use bevy_ecs::change_detection::DetectChangesMut;
use bevy_ecs::entity::Entities;
use bevy_ecs::entity::Entity;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::lifecycle::RemovedComponents;
use bevy_ecs::query::Changed;
use bevy_ecs::query::With;
use bevy_ecs::query::Without;
#[cfg(test)]
use bevy_ecs::schedule::IntoScheduleConfigs;
#[cfg(test)]
use bevy_ecs::schedule::Schedule;
use bevy_ecs::system::Local;
use bevy_ecs::system::Query;
use bevy_ecs::system::ResMut;
#[cfg(test)]
use bevy_ecs::world::World;
use bevy_math::Affine3A;
use bevy_math::Vec3;
use bevy_platform::collections::HashMap;
use bevy_transform::prelude::GlobalTransform;
use bevy_transform::prelude::Transform;

use crate::AnchorFrame;
use crate::AnchorPose;
use crate::AnchorSite;
#[cfg(test)]
use crate::AnchorSystems;
use crate::AnchoredTo;
use crate::AttachmentResolveAction;
use crate::AttachmentResolveCandidate;
use crate::AttachmentResolveDiagnostics;
use crate::AttachmentResolveReasons;
use crate::ResolvedAnchorGeometry;
use crate::ResolvedAnchorOffset;
use crate::ResolvedAnchorWorld;
use crate::attachment;
use crate::attachment::AttachmentResolverScratch;

#[cfg(test)]
#[path = "../fixtures.rs"]
#[allow(
    dead_code,
    reason = "shared geometry fixtures; the resolver tests use a subset"
)]
mod fixtures;

const ORTHONORMAL_EPSILON: f32 = 1e-4;
#[cfg(test)]
const TEST_QUAD_HEIGHT: f32 = 1.0;
#[cfg(test)]
const TEST_QUAD_WIDTH: f32 = 2.0;

/// Reason an anchor relationship did not resolve this frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum AnchorResolveSkip {
    /// The source entity lacks [`ResolvedAnchorGeometry`].
    MissingSourceGeometry,
    /// The target entity lacks [`ResolvedAnchorGeometry`].
    MissingTargetGeometry,
    /// The source entity lacks either [`Transform`] or [`GlobalTransform`].
    MissingSourceTransform,
    /// The target entity lacks [`GlobalTransform`].
    MissingTargetTransform,
    /// The source or target geometry does not contain this anchor id.
    MissingAnchor(AnchorSite),
    /// The target entity no longer exists.
    DespawnedTarget,
    /// The source's transform parent is missing or degenerate (non-finite or
    /// non-invertible), or the wanted pose cannot be expressed under it without skew.
    UnsupportedParentTransform,
    /// The source entity's current global scale is non-finite.
    NonFiniteScale,
    /// The source depends on a target that already skipped this frame.
    BlockedBySkippedDependency,
    /// The source participates in an anchor relationship cycle.
    Cycle,
    /// The source depends on a cycle in the anchor relationship graph.
    BlockedByCycle,
}

/// Resolver diagnostics for [`resolve_anchors`].
pub type AnchorResolveDiagnostics = AttachmentResolveDiagnostics<AnchorResolveSkip>;

/// Reusable temporary storage used exclusively by [`resolve_anchors`].
///
/// Applications do not insert or inspect this type. Bevy creates one
/// `Local<AnchorResolverScratch>` for each registered [`resolve_anchors`] system.
/// Every invocation reads the current relationships and geometry from ECS,
/// then empties its traversal collections before returning. The scratch also
/// retains only the semantic global-transform inputs of the last completed
/// pass, so equal-value transform ticks can be ignored.
#[derive(Default)]
pub struct AnchorResolverScratch {
    candidates:          Vec<AttachmentResolveCandidate<AnchorResolveSkip>>,
    resolution_order:    Vec<AttachmentResolveAction>,
    cached_placements:   Vec<CachedAnchorPlacement>,
    cached_indices:      HashMap<Entity, usize>,
    resolved_globals:    HashMap<Entity, GlobalTransform>,
    dirty_placements:    Vec<bool>,
    skipped_placements:  Vec<bool>,
    attachment_resolver: AttachmentResolverScratch,
    input_state:         AnchorResolveInputState,
}

impl AnchorResolverScratch {
    fn can_resolve_cached_pose(
        &self,
        evidence: &AnchorResolveInputEvidence,
        diagnostics: &AnchorResolveDiagnostics,
    ) -> bool {
        self.input_state
            .permits_cached_pose_resolution(evidence, diagnostics)
            && !self.resolution_order.is_empty()
            && self.cached_placements.len() == self.resolution_order.len()
    }
}

/// Inputs that stay fixed while only an anchor pose is animating.
#[derive(Clone, Copy)]
struct CachedAnchorPlacement {
    source:           Entity,
    target:           Entity,
    target_placement: Option<usize>,
    target_frame:     AnchorFrame,
    source_frame:     AnchorFrame,
    offset:           Vec3,
    source_scale:     Vec3,
    source_parent:    AnchorPlacementParent,
    parent_placement: Option<usize>,
    last_global:      GlobalTransform,
}

/// Coordinate space in which an anchored source's local transform is stored.
#[derive(Clone, Copy)]
enum AnchorPlacementParent {
    World,
    Entity(Entity),
}

/// The global transforms the last completed anchor pass actually read.
///
/// A component change tick alone is not enough evidence: transform
/// propagation and the word presentation can both write the same value the
/// resolver already solved against. Missing values are retained explicitly so
/// their later arrival opens the pass too.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ObservedAnchorGlobal {
    Missing,
    Present(GlobalTransform),
}

/// Semantic resolver inputs retained across invocations of one registered
/// system.
#[derive(Default)]
enum AnchorResolveInputState {
    #[default]
    NeverResolved,
    ResolvedAgainst(HashMap<Entity, ObservedAnchorGlobal>),
}

struct AnchorResolveInputEvidence {
    non_pose_component_changed: bool,
    changed_poses:              Vec<Entity>,
    changed_globals:            Vec<(Entity, GlobalTransform)>,
    removed_globals:            Vec<Entity>,
}

impl AnchorResolveInputState {
    fn requires_resolution(
        &self,
        evidence: &AnchorResolveInputEvidence,
        diagnostics: &AnchorResolveDiagnostics,
    ) -> bool {
        let Self::ResolvedAgainst(observed_globals) = self else {
            return true;
        };
        let meaningful_global_change = evidence.changed_globals.iter().any(|(entity, current)| {
            observed_globals
                .get(entity)
                .is_some_and(|observed| *observed != ObservedAnchorGlobal::Present(*current))
        });
        let meaningful_global_removal = evidence
            .removed_globals
            .iter()
            .any(|entity| observed_globals.contains_key(entity));

        evidence.non_pose_component_changed
            || !evidence.changed_poses.is_empty()
            || meaningful_global_removal
            || meaningful_global_change
            || diagnostics.current().next().is_some()
    }

    fn permits_cached_pose_resolution(
        &self,
        evidence: &AnchorResolveInputEvidence,
        diagnostics: &AnchorResolveDiagnostics,
    ) -> bool {
        let Self::ResolvedAgainst(observed_globals) = self else {
            return false;
        };
        let meaningful_global_change = evidence.changed_globals.iter().any(|(entity, current)| {
            observed_globals
                .get(entity)
                .is_some_and(|observed| *observed != ObservedAnchorGlobal::Present(*current))
        });
        let meaningful_global_removal = evidence
            .removed_globals
            .iter()
            .any(|entity| observed_globals.contains_key(entity));

        !evidence.non_pose_component_changed
            && !evidence.changed_poses.is_empty()
            && !meaningful_global_change
            && !meaningful_global_removal
            && diagnostics.current().next().is_none()
    }

    fn record_resolution(
        &mut self,
        anchored_entities: &Query<Entity, With<AnchoredTo>>,
        unanchored_geometry: &Query<Entity, (With<ResolvedAnchorGeometry>, Without<AnchoredTo>)>,
        anchored_parents: &Query<&ChildOf, With<AnchoredTo>>,
        globals: &Query<'_, '_, &GlobalTransform>,
        resolved_globals: &HashMap<Entity, GlobalTransform>,
    ) {
        if matches!(self, Self::NeverResolved) {
            *self = Self::ResolvedAgainst(HashMap::new());
        }
        let Self::ResolvedAgainst(observed_globals) = self else {
            return;
        };
        observed_globals.clear();
        for entity in anchored_entities {
            observed_globals.insert(entity, observed_global(entity, globals, resolved_globals));
        }
        for entity in unanchored_geometry {
            observed_globals.insert(entity, observed_global(entity, globals, resolved_globals));
        }
        for child_of in anchored_parents {
            let parent = child_of.parent();
            observed_globals.insert(parent, observed_global(parent, globals, resolved_globals));
        }
    }

    fn record_resolved_globals(&mut self, resolved_globals: &HashMap<Entity, GlobalTransform>) {
        let Self::ResolvedAgainst(observed_globals) = self else {
            return;
        };
        for (&entity, &global) in resolved_globals {
            observed_globals.insert(entity, ObservedAnchorGlobal::Present(global));
        }
    }
}

fn observed_global(
    entity: Entity,
    globals: &Query<&GlobalTransform>,
    resolved_globals: &HashMap<Entity, GlobalTransform>,
) -> ObservedAnchorGlobal {
    resolved_globals
        .get(&entity)
        .copied()
        .or_else(|| globals.get(entity).ok().copied())
        .map_or(ObservedAnchorGlobal::Missing, ObservedAnchorGlobal::Present)
}

/// Resolves [`AnchoredTo`] relationships into local [`Transform`] values.
///
/// `resolve_anchors` is the only system in this crate that writes `Transform`
/// for entities carrying `AnchoredTo`. Drivers should write [`AnchorPose`] in
/// [`AnchorSystems::AnimatePose`](crate::AnchorSystems::AnimatePose), and
/// consumers should run this system in
/// [`AnchorSystems::Resolve`](crate::AnchorSystems::Resolve) before
/// transform propagation.
///
/// The system reads `GlobalTransform` values produced by the previous
/// propagation pass. It writes local transforms, so external same-frame reads
/// of anchored entities' `GlobalTransform` components are one frame stale until
/// the consumer runs `TransformSystems::Propagate`.
///
/// Sources keep their hierarchy and compensate parent stretch when an exact
/// local translation, rotation and scale can represent the rigid world pose.
/// Degenerate parents and poses requiring local shear skip with
/// [`AnchorResolveSkip::UnsupportedParentTransform`].
///
/// [`ResolvedAnchorWorld`] is recomputed whenever a resolver input changes for
/// entities carrying the cache. Entities resolved in the current pass use the newly computed
/// `GlobalTransform` stored by `resolve_anchors`. Cache entries for entities
/// not resolved in the current frame use the `GlobalTransform` from the
/// previous propagation pass, matching every query read by the resolver because
/// [`AnchorSystems::Resolve`](crate::AnchorSystems::Resolve) runs before
/// `TransformSystems::Propagate`. The cache has the same freshness as the
/// resolve pass and is never change-detection-gated.
pub fn resolve_anchors(
    entities: &Entities,
    attachments: Query<(Entity, &AnchoredTo)>,
    geometry: Query<&ResolvedAnchorGeometry>,
    globals: Query<&GlobalTransform>,
    poses: Query<&AnchorPose>,
    offsets: Query<&ResolvedAnchorOffset>,
    parents: Query<&ChildOf>,
    mut transforms: Query<&mut Transform, With<AnchoredTo>>,
    mut anchor_worlds: Query<(
        Entity,
        &ResolvedAnchorGeometry,
        Option<&GlobalTransform>,
        &mut ResolvedAnchorWorld,
    )>,
    mut diagnostics: ResMut<AnchorResolveDiagnostics>,
    mut scratch: Local<AnchorResolverScratch>,
    changed_inputs: (
        Query<(), Changed<AnchoredTo>>,
        Query<Entity, Changed<AnchorPose>>,
        Query<(), Changed<ResolvedAnchorOffset>>,
        Query<(), Changed<ResolvedAnchorGeometry>>,
        Query<(), Changed<ChildOf>>,
    ),
    global_inputs: (
        Query<(Entity, &GlobalTransform), Changed<GlobalTransform>>,
        Query<Entity, With<AnchoredTo>>,
        Query<Entity, (With<ResolvedAnchorGeometry>, Without<AnchoredTo>)>,
        Query<&ChildOf, With<AnchoredTo>>,
    ),
    mut removed_inputs: (
        RemovedComponents<AnchoredTo>,
        RemovedComponents<AnchorPose>,
        RemovedComponents<ResolvedAnchorOffset>,
        RemovedComponents<ResolvedAnchorGeometry>,
        RemovedComponents<ChildOf>,
        RemovedComponents<GlobalTransform>,
    ),
) {
    let input_evidence = collect_anchor_input_evidence(
        &mut anchor_worlds,
        &mut transforms,
        &changed_inputs,
        &global_inputs,
        &mut removed_inputs,
    );
    if !scratch
        .input_state
        .requires_resolution(&input_evidence, &diagnostics)
    {
        return;
    }
    if scratch.can_resolve_cached_pose(&input_evidence, &diagnostics) {
        let AnchorResolverScratch {
            cached_placements,
            cached_indices,
            resolved_globals,
            dirty_placements,
            skipped_placements,
            input_state,
            ..
        } = &mut *scratch;
        resolve_cached_pose_changes(
            cached_placements,
            cached_indices,
            &input_evidence.changed_poses,
            &globals,
            &poses,
            &mut transforms,
            resolved_globals,
            dirty_placements,
            skipped_placements,
            &mut diagnostics,
        );
        refresh_resolved_anchor_world_cache(&mut anchor_worlds, resolved_globals);
        input_state.record_resolved_globals(resolved_globals);
        resolved_globals.clear();
        return;
    }
    let AnchorResolverScratch {
        candidates,
        resolution_order,
        cached_placements,
        cached_indices,
        resolved_globals,
        attachment_resolver,
        input_state,
        ..
    } = &mut *scratch;
    classify_candidates(
        candidates,
        entities,
        &attachments,
        &geometry,
        &globals,
        &transforms,
    );
    resolution_order.clear();
    cached_placements.clear();
    cached_indices.clear();
    resolved_globals.clear();
    attachment::resolve_attachments_with_scratch(
        candidates,
        resolve_reasons(),
        &mut diagnostics,
        attachment_resolver,
        |action| {
            if matches!(action, AttachmentResolveAction::Place { .. }) {
                resolution_order.push(action);
            }
            let result = handle_action(
                &geometry,
                &globals,
                &poses,
                &offsets,
                &parents,
                &mut transforms,
                resolved_globals,
                action,
            );
            cache_resolved_placement(cached_placements, cached_indices, result)
        },
    );
    refresh_anchor_world_cache(&mut anchor_worlds, resolved_globals);
    let (_, anchored_entities, unanchored_geometry, anchored_parents) = &global_inputs;
    input_state.record_resolution(
        anchored_entities,
        unanchored_geometry,
        anchored_parents,
        &globals,
        resolved_globals,
    );
    resolved_globals.clear();
}

fn cache_resolved_placement(
    cached_placements: &mut Vec<CachedAnchorPlacement>,
    cached_indices: &mut HashMap<Entity, usize>,
    result: Result<Option<CachedAnchorPlacement>, AnchorResolveSkip>,
) -> Result<(), AnchorResolveSkip> {
    if let Some(mut placement) = result? {
        placement.target_placement = cached_indices.get(&placement.target).copied();
        if let AnchorPlacementParent::Entity(parent) = placement.source_parent {
            placement.parent_placement = cached_indices.get(&parent).copied();
        }
        cached_indices.insert(placement.source, cached_placements.len());
        cached_placements.push(placement);
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "Bevy groups the resolver's change-detection queries by input role"
)]
fn collect_anchor_input_evidence(
    anchor_worlds: &mut Query<(
        Entity,
        &ResolvedAnchorGeometry,
        Option<&GlobalTransform>,
        &mut ResolvedAnchorWorld,
    )>,
    transforms: &mut Query<&mut Transform, With<AnchoredTo>>,
    changed_inputs: &(
        Query<(), Changed<AnchoredTo>>,
        Query<Entity, Changed<AnchorPose>>,
        Query<(), Changed<ResolvedAnchorOffset>>,
        Query<(), Changed<ResolvedAnchorGeometry>>,
        Query<(), Changed<ChildOf>>,
    ),
    global_inputs: &(
        Query<(Entity, &GlobalTransform), Changed<GlobalTransform>>,
        Query<Entity, With<AnchoredTo>>,
        Query<Entity, (With<ResolvedAnchorGeometry>, Without<AnchoredTo>)>,
        Query<&ChildOf, With<AnchoredTo>>,
    ),
    removed_inputs: &mut (
        RemovedComponents<AnchoredTo>,
        RemovedComponents<AnchorPose>,
        RemovedComponents<ResolvedAnchorOffset>,
        RemovedComponents<ResolvedAnchorGeometry>,
        RemovedComponents<ChildOf>,
        RemovedComponents<GlobalTransform>,
    ),
) -> AnchorResolveInputEvidence {
    let (changed_attachments, changed_poses, changed_offsets, changed_geometry, changed_parents) =
        changed_inputs;
    let (changed_globals, _, _, _) = global_inputs;
    let (
        removed_attachments,
        removed_poses,
        removed_offsets,
        removed_geometry,
        removed_parents,
        removed_globals,
    ) = removed_inputs;
    let new_anchor_world = anchor_worlds
        .iter_mut()
        .any(|(_, _, _, anchor_world)| anchor_world.is_added());
    AnchorResolveInputEvidence {
        non_pose_component_changed: new_anchor_world
            || !changed_attachments.is_empty()
            || !changed_offsets.is_empty()
            || !changed_geometry.is_empty()
            || !changed_parents.is_empty()
            || transforms
                .iter_mut()
                .any(|transform| transform.is_changed())
            || removed_attachments.read().next().is_some()
            || removed_poses.read().next().is_some()
            || removed_offsets.read().next().is_some()
            || removed_geometry.read().next().is_some()
            || removed_parents.read().next().is_some(),
        changed_poses:              changed_poses.iter().collect(),
        changed_globals:            changed_globals
            .iter()
            .map(|(entity, global)| (entity, *global))
            .collect(),
        removed_globals:            removed_globals.read().collect(),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the cached pass receives each reusable resolver buffer separately"
)]
fn resolve_cached_pose_changes(
    cached_placements: &mut [CachedAnchorPlacement],
    cached_indices: &HashMap<Entity, usize>,
    changed_poses: &[Entity],
    globals: &Query<&GlobalTransform>,
    poses: &Query<&AnchorPose>,
    transforms: &mut Query<&mut Transform, With<AnchoredTo>>,
    resolved_globals: &mut HashMap<Entity, GlobalTransform>,
    dirty_placements: &mut Vec<bool>,
    skipped_placements: &mut Vec<bool>,
    diagnostics: &mut AnchorResolveDiagnostics,
) {
    diagnostics.begin_frame();
    resolved_globals.clear();
    dirty_placements.resize(cached_placements.len(), false);
    dirty_placements.fill(false);
    for entity in changed_poses {
        if let Some(&index) = cached_indices.get(entity) {
            dirty_placements[index] = true;
        }
    }
    skipped_placements.resize(cached_placements.len(), false);
    skipped_placements.fill(false);

    for index in 0..cached_placements.len() {
        let placement_inputs = cached_placements[index];
        let source = placement_inputs.source;
        let target = placement_inputs.target;
        let target_dirty = placement_inputs
            .target_placement
            .is_some_and(|target_index| dirty_placements[target_index]);
        let parent_dirty = placement_inputs
            .parent_placement
            .is_some_and(|parent_index| dirty_placements[parent_index]);
        if !dirty_placements[index] && !target_dirty && !parent_dirty {
            continue;
        }
        dirty_placements[index] = true;
        if placement_inputs
            .target_placement
            .is_some_and(|target_index| skipped_placements[target_index])
        {
            skipped_placements[index] = true;
            diagnostics.record(
                source,
                target,
                AnchorResolveSkip::BlockedBySkippedDependency,
            );
            continue;
        }
        match cached_anchor_placement(cached_placements, index, globals, poses) {
            Ok(placement) => {
                let Ok(mut transform) = transforms.get_mut(source) else {
                    skipped_placements[index] = true;
                    diagnostics.record(source, target, AnchorResolveSkip::MissingSourceTransform);
                    continue;
                };
                transform.set_if_neq(placement.local_transform);
                cached_placements[index].last_global = placement.global_transform;
                resolved_globals.insert(source, placement.global_transform);
            },
            Err(reason) => {
                skipped_placements[index] = true;
                diagnostics.record(source, target, reason);
            },
        }
    }
    diagnostics.finish_frame();
}

fn classify_candidates(
    candidates: &mut Vec<AttachmentResolveCandidate<AnchorResolveSkip>>,
    entities: &Entities,
    attachments: &Query<(Entity, &AnchoredTo)>,
    geometry: &Query<&ResolvedAnchorGeometry>,
    globals: &Query<&GlobalTransform>,
    transforms: &Query<&mut Transform, With<AnchoredTo>>,
) {
    candidates.clear();
    candidates.extend(attachments.iter().map(|(source, attachment)| {
        let attachment = *attachment;
        let target = attachment.target();
        match validate_candidate(entities, geometry, globals, transforms, source, attachment) {
            Ok(()) => AttachmentResolveCandidate::Active {
                source,
                target,
                attachment,
            },
            Err(reason) => AttachmentResolveCandidate::Skipped {
                source,
                target,
                reason,
            },
        }
    }));
}

fn validate_candidate(
    entities: &Entities,
    geometry: &Query<&ResolvedAnchorGeometry>,
    globals: &Query<&GlobalTransform>,
    transforms: &Query<&mut Transform, With<AnchoredTo>>,
    source: Entity,
    attachment: AnchoredTo,
) -> Result<(), AnchorResolveSkip> {
    let target = attachment.target();
    if !entities.contains_spawned(target) {
        return Err(AnchorResolveSkip::DespawnedTarget);
    }
    if !geometry.contains(source) {
        return Err(AnchorResolveSkip::MissingSourceGeometry);
    }
    if !geometry.contains(target) {
        return Err(AnchorResolveSkip::MissingTargetGeometry);
    }
    if !transforms.contains(source) || !globals.contains(source) {
        return Err(AnchorResolveSkip::MissingSourceTransform);
    }
    if !globals.contains(target) {
        return Err(AnchorResolveSkip::MissingTargetTransform);
    }
    Ok(())
}

const fn resolve_reasons() -> AttachmentResolveReasons<AnchorResolveSkip> {
    AttachmentResolveReasons {
        blocked_by_skipped_dependency: AnchorResolveSkip::BlockedBySkippedDependency,
        cycle:                         AnchorResolveSkip::Cycle,
        blocked_by_cycle:              AnchorResolveSkip::BlockedByCycle,
    }
}

fn handle_action(
    geometry: &Query<&ResolvedAnchorGeometry>,
    globals: &Query<&GlobalTransform>,
    poses: &Query<&AnchorPose>,
    offsets: &Query<&ResolvedAnchorOffset>,
    parents: &Query<&ChildOf>,
    transforms: &mut Query<&mut Transform, With<AnchoredTo>>,
    resolved_globals: &mut HashMap<Entity, GlobalTransform>,
    action: AttachmentResolveAction,
) -> Result<Option<CachedAnchorPlacement>, AnchorResolveSkip> {
    match action {
        AttachmentResolveAction::Place {
            source,
            target,
            attachment,
        } => place_anchor(
            geometry,
            globals,
            poses,
            offsets,
            parents,
            transforms,
            resolved_globals,
            source,
            target,
            attachment,
        ),
        AttachmentResolveAction::Fallback { source: _ } => Ok(None),
    }
}

fn place_anchor(
    geometry: &Query<&ResolvedAnchorGeometry>,
    globals: &Query<&GlobalTransform>,
    poses: &Query<&AnchorPose>,
    offsets: &Query<&ResolvedAnchorOffset>,
    parents: &Query<&ChildOf>,
    transforms: &mut Query<&mut Transform, With<AnchoredTo>>,
    resolved_globals: &mut HashMap<Entity, GlobalTransform>,
    source: Entity,
    target: Entity,
    attachment: AnchoredTo,
) -> Result<Option<CachedAnchorPlacement>, AnchorResolveSkip> {
    let mut placement_inputs = anchor_placement_inputs(
        geometry,
        globals,
        offsets,
        parents,
        resolved_globals,
        source,
        target,
        attachment,
    )?;
    let placement = anchor_placement(globals, poses, resolved_globals, placement_inputs)?;
    let Ok(mut transform) = transforms.get_mut(source) else {
        return Err(AnchorResolveSkip::MissingSourceTransform);
    };
    transform.set_if_neq(placement.local_transform);
    resolved_globals.insert(source, placement.global_transform);
    placement_inputs.last_global = placement.global_transform;
    Ok(Some(placement_inputs))
}

fn cached_anchor_placement(
    cached_placements: &[CachedAnchorPlacement],
    index: usize,
    globals: &Query<&GlobalTransform>,
    poses: &Query<&AnchorPose>,
) -> Result<AnchorPlacement, AnchorResolveSkip> {
    let inputs = cached_placements[index];
    let target_global = match inputs.target_placement {
        Some(target_index) => cached_placements[target_index].last_global,
        None => globals
            .get(inputs.target)
            .copied()
            .map_err(|_| AnchorResolveSkip::MissingTargetTransform)?,
    };
    let parent_global = match inputs.source_parent {
        AnchorPlacementParent::World => None,
        AnchorPlacementParent::Entity(parent) => Some(match inputs.parent_placement {
            Some(parent_index) => cached_placements[parent_index].last_global,
            None => globals
                .get(parent)
                .copied()
                .map_err(|_| AnchorResolveSkip::UnsupportedParentTransform)?,
        }),
    };
    anchor_placement_from_globals(poses, inputs, target_global, parent_global)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AnchorPlacement {
    global_transform: GlobalTransform,
    local_transform:  Transform,
}

fn anchor_placement_inputs(
    geometry: &Query<&ResolvedAnchorGeometry>,
    globals: &Query<&GlobalTransform>,
    offsets: &Query<&ResolvedAnchorOffset>,
    parents: &Query<&ChildOf>,
    resolved_globals: &HashMap<Entity, GlobalTransform>,
    source: Entity,
    target: Entity,
    attachment: AnchoredTo,
) -> Result<CachedAnchorPlacement, AnchorResolveSkip> {
    let source_global = entity_global(globals, resolved_globals, source)
        .ok_or(AnchorResolveSkip::MissingSourceTransform)?;
    let parent = parents.get(source).ok().map(ChildOf::parent);
    // The source's x axis is the size reference. Parent stretch must not turn
    // anchored content into a stretched or sheared world frame.
    let source_scale = Vec3::splat(source_global.affine().matrix3.x_axis.length());
    if !source_scale.is_finite() || !source_global.affine().matrix3.is_finite() {
        if let Some(parent) = parent {
            let parent_global = entity_global(globals, resolved_globals, parent)
                .ok_or(AnchorResolveSkip::UnsupportedParentTransform)?;
            validate_supported_parent_transform(&parent_global)?;
        }
        return Err(AnchorResolveSkip::NonFiniteScale);
    }

    let target_frame = anchor_frame(geometry, target, attachment.target_anchor())?;
    let source_frame = anchor_frame(geometry, source, attachment.source_anchor())?;
    let offset = offsets
        .get(source)
        .copied()
        .map_or_else(|_| attachment.offset(), ResolvedAnchorOffset::offset)
        .into_inner();
    let source_parent = parent.map_or(AnchorPlacementParent::World, AnchorPlacementParent::Entity);

    Ok(CachedAnchorPlacement {
        source,
        target,
        target_placement: None,
        target_frame,
        source_frame,
        offset,
        source_scale,
        source_parent,
        parent_placement: None,
        last_global: source_global,
    })
}

fn anchor_placement(
    globals: &Query<&GlobalTransform>,
    poses: &Query<&AnchorPose>,
    resolved_globals: &HashMap<Entity, GlobalTransform>,
    inputs: CachedAnchorPlacement,
) -> Result<AnchorPlacement, AnchorResolveSkip> {
    let target_global = entity_global(globals, resolved_globals, inputs.target)
        .ok_or(AnchorResolveSkip::MissingTargetTransform)?;
    let parent_global = match inputs.source_parent {
        AnchorPlacementParent::World => None,
        AnchorPlacementParent::Entity(parent) => Some(
            entity_global(globals, resolved_globals, parent)
                .ok_or(AnchorResolveSkip::UnsupportedParentTransform)?,
        ),
    };
    anchor_placement_from_globals(poses, inputs, target_global, parent_global)
}

fn anchor_placement_from_globals(
    poses: &Query<&AnchorPose>,
    inputs: CachedAnchorPlacement,
    target_global: GlobalTransform,
    parent_global: Option<GlobalTransform>,
) -> Result<AnchorPlacement, AnchorResolveSkip> {
    let pose = poses.get(inputs.source).copied().unwrap_or_default();

    let target_world = target_global.transform_point(inputs.target_frame.position().into_inner());
    let base =
        target_global.rotation().normalize() * inputs.target_frame.orientation().into_inner();
    let rotation = base
        * pose.rotation.into_inner()
        * inputs.source_frame.orientation().inverse().into_inner();
    let translation = target_world + base * (inputs.offset + pose.translation.into_inner())
        - rotation * (inputs.source_scale * inputs.source_frame.position().into_inner());
    let global_transform = GlobalTransform::from(Transform {
        translation,
        rotation,
        scale: inputs.source_scale,
    });
    let local_transform =
        local_transform_for(inputs.source_parent, parent_global, global_transform)?;

    Ok(AnchorPlacement {
        global_transform,
        local_transform,
    })
}

fn entity_global(
    globals: &Query<&GlobalTransform>,
    resolved_globals: &HashMap<Entity, GlobalTransform>,
    entity: Entity,
) -> Option<GlobalTransform> {
    resolved_globals
        .get(&entity)
        .copied()
        .or_else(|| globals.get(entity).ok().copied())
}

fn anchor_frame(
    geometry: &Query<&ResolvedAnchorGeometry>,
    entity: Entity,
    anchor_site: AnchorSite,
) -> Result<AnchorFrame, AnchorResolveSkip> {
    geometry
        .get(entity)
        .map_err(|_| AnchorResolveSkip::MissingSourceGeometry)?
        .frame(anchor_site)
        .copied()
        .map_err(|_| AnchorResolveSkip::MissingAnchor(anchor_site))
}

fn local_transform_for(
    parent: AnchorPlacementParent,
    parent_global: Option<GlobalTransform>,
    global_transform: GlobalTransform,
) -> Result<Transform, AnchorResolveSkip> {
    let AnchorPlacementParent::Entity(_) = parent else {
        return Ok(global_transform.compute_transform());
    };
    let parent_global = parent_global.ok_or(AnchorResolveSkip::UnsupportedParentTransform)?;
    local_transform_for_parent(&parent_global, &global_transform)
}

/// Expresses a world pose below a parent without losing shear during decomposition.
///
/// Rigid sources can remain children of stretched parents when an exact local
/// TRS exists. Degenerate parents and poses requiring local shear are refused.
///
/// # Errors
/// Returns [`AnchorResolveSkip::UnsupportedParentTransform`] when the local
/// transform cannot reproduce the requested world pose without shear.
pub fn local_transform_for_parent(
    parent_global: &GlobalTransform,
    global_transform: &GlobalTransform,
) -> Result<Transform, AnchorResolveSkip> {
    let parent_inverse = validate_supported_parent_transform(parent_global)?;
    let relative = parent_inverse * global_transform.affine();
    let (scale, rotation, translation) = relative.to_scale_rotation_translation();
    let local = Transform {
        translation,
        rotation,
        scale,
    };
    let reconstructed = parent_global.mul_transform(local);
    let wanted = global_transform.affine();
    let actual = reconstructed.affine();
    // Decomposition alone silently loses shear. Only write a local transform
    // when recomposition recovers every world axis and the world translation.
    let tolerance = ORTHONORMAL_EPSILON * wanted.matrix3.x_axis.length();
    if [
        (actual.matrix3.x_axis - wanted.matrix3.x_axis).length(),
        (actual.matrix3.y_axis - wanted.matrix3.y_axis).length(),
        (actual.matrix3.z_axis - wanted.matrix3.z_axis).length(),
    ]
    .into_iter()
    .all(|error| error <= tolerance)
        && (actual.translation - wanted.translation).length()
            <= ORTHONORMAL_EPSILON * wanted.translation.length().max(1.0)
    {
        return Ok(local);
    }
    Err(AnchorResolveSkip::UnsupportedParentTransform)
}

fn validate_supported_parent_transform(
    parent: &GlobalTransform,
) -> Result<Affine3A, AnchorResolveSkip> {
    let affine = parent.affine();
    let inverse = affine.inverse();
    if !affine.is_finite() || !inverse.is_finite() {
        return Err(AnchorResolveSkip::UnsupportedParentTransform);
    }
    Ok(inverse)
}

fn refresh_anchor_world_cache(
    anchor_worlds: &mut Query<(
        Entity,
        &ResolvedAnchorGeometry,
        Option<&GlobalTransform>,
        &mut ResolvedAnchorWorld,
    )>,
    resolved_globals: &HashMap<Entity, GlobalTransform>,
) {
    for (entity, geometry, global_transform, mut anchor_world) in anchor_worlds.iter_mut() {
        let Some(global_transform) = resolved_globals
            .get(&entity)
            .copied()
            .or_else(|| global_transform.copied())
        else {
            anchor_world.points.clear();
            continue;
        };
        anchor_world
            .points
            .retain(|anchor_id, _| geometry.frame(*anchor_id).is_ok());
        for (anchor_id, point) in geometry.frames() {
            anchor_world.points.insert(
                *anchor_id,
                global_transform.transform_point(point.position().into_inner()),
            );
        }
    }
}

fn refresh_resolved_anchor_world_cache(
    anchor_worlds: &mut Query<(
        Entity,
        &ResolvedAnchorGeometry,
        Option<&GlobalTransform>,
        &mut ResolvedAnchorWorld,
    )>,
    resolved_globals: &HashMap<Entity, GlobalTransform>,
) {
    for (&entity, &global_transform) in resolved_globals {
        let Ok((_, geometry, _, mut anchor_world)) = anchor_worlds.get_mut(entity) else {
            continue;
        };
        anchor_world
            .points
            .retain(|anchor_id, _| geometry.frame(*anchor_id).is_ok());
        for (anchor_id, point) in geometry.frames() {
            anchor_world.points.insert(
                *anchor_id,
                global_transform.transform_point(point.position().into_inner()),
            );
        }
    }
}

/// Test helper that creates a world with resolver diagnostics installed.
#[cfg(test)]
pub(crate) fn world_with_diagnostics() -> World {
    let mut world = World::new();
    world.insert_resource(AnchorResolveDiagnostics::default());
    world
}

/// Test helper that runs only [`resolve_anchors`].
#[cfg(test)]
fn run_resolve(world: &mut World) {
    let mut schedule = Schedule::default();
    schedule.add_systems(resolve_anchors.in_set(AnchorSystems::Resolve));
    schedule.run(world);
}

/// Test helper that spawns one flat quad with transform components.
#[cfg(test)]
pub(crate) fn spawn_quad(world: &mut World, transform: Transform) -> Entity {
    world
        .spawn((quad_geometry(), transform, GlobalTransform::from(transform)))
        .id()
}

/// Test helper that returns the canonical flat quad geometry.
#[cfg(test)]
fn quad_geometry() -> ResolvedAnchorGeometry {
    fixtures::quad_geometry(TEST_QUAD_WIDTH, TEST_QUAD_HEIGHT)
}

#[cfg(test)]
mod tests {
    use bevy_ecs::entity::Entity;
    use bevy_ecs::hierarchy::ChildOf;
    use bevy_ecs::prelude::Query;
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use bevy_ecs::schedule::Schedule;
    use bevy_ecs::world::World;
    use bevy_math::Quat;
    use bevy_math::Vec3;
    use bevy_transform::prelude::GlobalTransform;
    use bevy_transform::prelude::Transform;

    use super::AnchorResolveDiagnostics;
    use super::AnchorResolveSkip;
    use super::quad_geometry;
    use super::resolve_anchors;
    use super::run_resolve;
    use super::spawn_quad;
    use super::world_with_diagnostics;
    use crate::AnchorFrame;
    use crate::AnchorPose;
    use crate::AnchorSite;
    use crate::AnchorSystems;
    use crate::AnchoredTo;
    use crate::AttachmentResolveDiagnostics;
    use crate::Displacement;
    use crate::Orientation;
    use crate::Position;
    use crate::ResolvedAnchorGeometry;
    use crate::ResolvedAnchorOffset;
    use crate::ResolvedAnchorWorld;

    const ASSERT_EPSILON: f32 = 1e-4;
    const CHAIN_LIFT: f32 = 1.0;
    const CHILD_SCALE: f32 = 0.5;
    const OFFSET_OVERRIDE_Y: f32 = 2.0;
    const OFFSET_TRACE_X: f32 = 0.25;
    const OFFSET_TRACE_Y: f32 = -0.5;
    const PIVOT_OFFSET: Vec3 = Vec3::new(0.0, 0.0, 0.25);
    const POSE_LIFT: f32 = 0.5;
    const QUAD_HEIGHT: f32 = 1.0;
    const QUAD_WIDTH: f32 = 2.0;
    const SEMANTIC_OFFSET: Vec3 = Vec3::new(0.25, -0.5, 0.75);
    const SEMANTIC_TARGET: Vec3 = Vec3::new(3.0, 2.0, 1.0);
    const TARGET_ANCHOR_X: f32 = 2.0;
    const TARGET_ANCHOR_Y: f32 = 1.0;
    const TARGET_TRACE_X: f32 = 3.0;
    const TARGET_TRACE_Y: f32 = 3.0;

    #[test]
    fn two_quads_top_left_to_top_right() {
        let mut world = world_with_diagnostics();
        let target = spawn_quad(&mut world, Transform::default());
        let source = spawn_quad(&mut world, Transform::default());
        world.entity_mut(source).insert(AnchoredTo::new(
            target,
            AnchorSite::Vertex(0),
            AnchorSite::Vertex(1),
        ));

        run_resolve(&mut world);

        assert_anchor_matches(
            &world,
            source,
            AnchorSite::Vertex(0),
            target,
            AnchorSite::Vertex(1),
        );
    }

    #[test]
    fn successive_runs_reread_geometry_that_was_missing() {
        let mut world = world_with_diagnostics();
        let target_translation = Vec3::new(TARGET_ANCHOR_X, TARGET_ANCHOR_Y, 0.0);
        let target =
            spawn_transform_only(&mut world, Transform::from_translation(target_translation));
        let source = spawn_quad(
            &mut world,
            Transform::from_translation(Vec3::new(-1.0, -1.0, 0.0)),
        );
        world.entity_mut(source).insert(AnchoredTo::new(
            target,
            AnchorSite::Center,
            AnchorSite::Center,
        ));
        let mut schedule = Schedule::default();
        schedule.add_systems(resolve_anchors.in_set(AnchorSystems::Resolve));

        schedule.run(&mut world);

        assert_current_diagnostic(
            &world,
            source,
            target,
            AnchorResolveSkip::MissingTargetGeometry,
        );

        world.entity_mut(target).insert(quad_geometry());
        schedule.run(&mut world);

        assert_close_vec3(transform(&world, source).translation, target_translation);
        assert!(diagnostics(&world).current().next().is_none());
    }

    #[test]
    fn missing_runtime_input_preserves_authored_relationship() {
        let mut world = world_with_diagnostics();
        let missing_target = world.spawn_empty().id();
        let source = spawn_quad(&mut world, Transform::from_translation(Vec3::X));
        let attachment = AnchoredTo::new(missing_target, AnchorSite::Center, AnchorSite::Center);
        world.entity_mut(source).insert(attachment);

        run_resolve(&mut world);

        assert_eq!(world.get::<AnchoredTo>(source), Some(&attachment));
        assert_current_diagnostic(
            &world,
            source,
            missing_target,
            AnchorResolveSkip::MissingTargetGeometry,
        );
    }

    #[test]
    fn nonzero_semantic_offset_resolves_in_target_frame() {
        let mut world = world_with_diagnostics();
        let target = spawn_quad(&mut world, Transform::from_translation(SEMANTIC_TARGET));
        let source = spawn_quad(&mut world, Transform::default());
        world.entity_mut(source).insert(
            AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center)
                .with_offset(Displacement::from(SEMANTIC_OFFSET)),
        );

        run_resolve(&mut world);

        assert_close_vec3(
            transform(&world, source).translation,
            SEMANTIC_TARGET + SEMANTIC_OFFSET,
        );
    }

    #[test]
    fn offset_trace_applies_raw_offset_in_target_frame() {
        let mut world = world_with_diagnostics();
        let target = spawn_quad(
            &mut world,
            Transform::from_translation(Vec3::new(TARGET_TRACE_X, TARGET_TRACE_Y, 0.0)),
        );
        let source = spawn_quad(&mut world, Transform::default());
        world.entity_mut(source).insert((
            AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center).with_offset(
                Displacement::from(Vec3::new(OFFSET_TRACE_X, OFFSET_TRACE_Y, 0.0)),
            ),
            ResolvedAnchorWorld::default(),
        ));

        run_resolve(&mut world);

        let expected = Vec3::new(
            TARGET_TRACE_X + OFFSET_TRACE_X,
            TARGET_TRACE_Y + OFFSET_TRACE_Y,
            0.0,
        );
        assert_close_vec3(
            world_anchor_point(&world, source, AnchorSite::Center),
            expected,
        );
        assert_close_vec3(
            cached_anchor_point(&world, source, AnchorSite::Center),
            expected,
        );
    }

    #[test]
    fn scale_and_parent_rotation_port_matches_world_anchor_expectation() {
        let mut world = world_with_diagnostics();
        let parent_transform =
            Transform::from_rotation(Quat::from_rotation_z(core::f32::consts::FRAC_PI_2));
        let parent = spawn_transform_only(&mut world, parent_transform);
        let target_transform = Transform::from_translation(Vec3::new(
            TARGET_ANCHOR_X + QUAD_WIDTH / 2.0,
            TARGET_ANCHOR_Y - QUAD_HEIGHT / 2.0,
            0.0,
        ));
        let target = spawn_quad(&mut world, target_transform);
        let source_transform = Transform::from_scale(Vec3::splat(CHILD_SCALE));
        let source_global = global_transform(&world, parent).mul_transform(source_transform);
        let source = world
            .spawn((
                quad_geometry(),
                source_transform,
                source_global,
                ChildOf(parent),
                AnchoredTo::new(target, AnchorSite::Vertex(2), AnchorSite::Vertex(0)),
            ))
            .id();

        run_resolve(&mut world);

        let actual = global_transform(&world, parent).mul_transform(transform(&world, source));
        let (scale, rotation, translation) = actual.to_scale_rotation_translation();
        assert_close_vec3(translation, Vec3::new(1.5, 1.25, 0.0));
        assert_close_quat(rotation, Quat::IDENTITY);
        assert_close_vec3(scale, Vec3::splat(CHILD_SCALE));
    }

    #[test]
    fn pose_written_in_animation_set_lands_this_frame() {
        let mut world = world_with_diagnostics();
        let target = spawn_quad(
            &mut world,
            Transform::from_translation(Vec3::new(TARGET_ANCHOR_X, TARGET_ANCHOR_Y, 0.0)),
        );
        let source = spawn_quad(&mut world, Transform::default());
        world.entity_mut(source).insert((
            AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
            AnchorPose::default(),
        ));
        let mut schedule = Schedule::default();
        schedule.configure_sets((AnchorSystems::AnimatePose, AnchorSystems::Resolve).chain());
        schedule.add_systems((
            lift_pose.in_set(AnchorSystems::AnimatePose),
            resolve_anchors.in_set(AnchorSystems::Resolve),
        ));

        schedule.run(&mut world);

        assert_close_vec3(
            world_anchor_point(&world, source, AnchorSite::Center),
            world_anchor_point(&world, target, AnchorSite::Center) + Vec3::Z * POSE_LIFT,
        );
    }

    #[test]
    fn pose_translation_composes_with_authored_offset() {
        let mut world = world_with_diagnostics();
        let target = spawn_quad(&mut world, Transform::default());
        let source = spawn_quad(&mut world, Transform::default());
        let authored_offset = Vec3::new(0.25, -0.5, 0.75);
        world.entity_mut(source).insert((
            AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center)
                .with_offset(Displacement::from(authored_offset)),
            AnchorPose {
                rotation:    Orientation::default(),
                translation: Displacement::from(PIVOT_OFFSET),
            },
        ));
        let mut schedule = Schedule::default();
        schedule.add_systems(resolve_anchors.in_set(AnchorSystems::Resolve));

        schedule.run(&mut world);

        assert_close_vec3(
            world_anchor_point(&world, source, AnchorSite::Center),
            world_anchor_point(&world, target, AnchorSite::Center) + authored_offset + PIVOT_OFFSET,
        );
    }

    #[test]
    fn frame_seating_preserves_pin_and_composes_source_frame_out() {
        let mut world = world_with_diagnostics();
        let target_frame = Quat::from_rotation_z(core::f32::consts::FRAC_PI_2);
        let source_frame = Quat::from_rotation_x(core::f32::consts::FRAC_PI_2);
        let pose_rotation = Quat::from_rotation_y(core::f32::consts::FRAC_PI_2);
        let target = spawn_geometry(
            &mut world,
            framed_geometry(AnchorSite::Center, Vec3::ZERO, target_frame),
            Transform::default(),
        );
        let source = spawn_geometry(
            &mut world,
            framed_geometry(
                AnchorSite::Vertex(0),
                Vec3::new(-1.0, 1.0, 0.0),
                source_frame,
            ),
            Transform::default(),
        );
        world.entity_mut(source).insert((
            AnchoredTo::new(target, AnchorSite::Vertex(0), AnchorSite::Center),
            AnchorPose {
                rotation:    valid(Orientation::try_from(pose_rotation)),
                translation: Displacement::default(),
            },
        ));

        run_resolve(&mut world);

        let transform = transform(&world, source);
        assert_anchor_matches(
            &world,
            source,
            AnchorSite::Vertex(0),
            target,
            AnchorSite::Center,
        );
        assert_close_quat(
            transform.rotation * source_frame,
            target_frame * pose_rotation,
        );
    }

    #[test]
    fn wide_and_deep_tree_resolves_in_topological_order() {
        let mut world = world_with_diagnostics();
        let root = spawn_quad(&mut world, Transform::default());
        let first = spawn_center_anchor(&mut world, root, Vec3::X);
        let second = spawn_center_anchor(&mut world, first, Vec3::Y * CHAIN_LIFT);
        let third = spawn_center_anchor(&mut world, second, Vec3::Y * CHAIN_LIFT);
        let fourth = spawn_center_anchor(&mut world, third, Vec3::Y * CHAIN_LIFT);
        let fanout_offsets = [
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(-2.0, 0.0, 0.0),
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(0.0, -2.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(2.0, -1.0, 0.0),
            Vec3::new(1.0, -2.0, 0.0),
        ];
        let fanout = fanout_offsets
            .into_iter()
            .map(|offset| spawn_center_anchor(&mut world, root, offset))
            .collect::<Vec<_>>();

        run_resolve(&mut world);

        assert_close_vec3(
            world_anchor_point(&world, fourth, AnchorSite::Center),
            Vec3::new(1.0, 3.0, 0.0),
        );
        assert_close_vec3(
            world_anchor_point(&world, fanout[fanout.len() - 1], AnchorSite::Center),
            Vec3::new(1.0, -2.0, 0.0),
        );
        assert!(diagnostics(&world).is_empty());
    }

    #[test]
    fn resolved_anchor_offset_override_beats_anchored_to_offset() {
        let mut world = world_with_diagnostics();
        let target = spawn_quad(&mut world, Transform::default());
        let source = spawn_quad(&mut world, Transform::default());
        world.entity_mut(source).insert((
            AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center)
                .with_offset(Displacement::from(Vec3::new(10.0, 0.0, 0.0))),
            ResolvedAnchorOffset::from(Displacement::from(Vec3::new(0.0, OFFSET_OVERRIDE_Y, 0.0))),
        ));

        run_resolve(&mut world);

        assert_close_vec3(
            world_anchor_point(&world, source, AnchorSite::Center),
            Vec3::new(0.0, OFFSET_OVERRIDE_Y, 0.0),
        );
    }

    #[test]
    fn degenerate_transform_parent_records_skip() {
        let mut world = world_with_diagnostics();
        let parent =
            spawn_transform_only(&mut world, Transform::from_scale(Vec3::new(2.0, 0.0, 1.0)));
        let target = spawn_quad(&mut world, Transform::default());
        let source = world
            .spawn((
                quad_geometry(),
                Transform::default(),
                global_transform(&world, parent),
                ChildOf(parent),
                AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
            ))
            .id();

        run_resolve(&mut world);

        assert_eq!(transform(&world, source), Transform::default());
        assert_current_diagnostic(
            &world,
            source,
            target,
            AnchorResolveSkip::UnsupportedParentTransform,
        );
    }

    fn lift_pose(mut poses: Query<&mut AnchorPose>) {
        for mut pose in &mut poses {
            pose.translation = Displacement::from(Vec3::Z * POSE_LIFT);
        }
    }

    fn spawn_transform_only(world: &mut World, transform: Transform) -> Entity {
        world
            .spawn((transform, GlobalTransform::from(transform)))
            .id()
    }

    fn spawn_geometry(
        world: &mut World,
        geometry: ResolvedAnchorGeometry,
        transform: Transform,
    ) -> Entity {
        world
            .spawn((geometry, transform, GlobalTransform::from(transform)))
            .id()
    }

    fn spawn_center_anchor(world: &mut World, target: Entity, offset: Vec3) -> Entity {
        let source = spawn_quad(world, Transform::default());
        world.entity_mut(source).insert(
            AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center)
                .with_offset(Displacement::from(offset)),
        );
        source
    }

    fn framed_geometry(
        anchor_site: AnchorSite,
        position: Vec3,
        frame: Quat,
    ) -> ResolvedAnchorGeometry {
        valid(ResolvedAnchorGeometry::try_new(
            [(
                anchor_site,
                valid(AnchorFrame::try_new(
                    Position::from(position),
                    valid(Orientation::try_from(frame)),
                )),
            )],
            [],
        ))
    }

    fn assert_anchor_matches(
        world: &World,
        source: Entity,
        source_anchor: AnchorSite,
        target: Entity,
        target_anchor: AnchorSite,
    ) {
        assert_close_vec3(
            world_anchor_point(world, source, source_anchor),
            world_anchor_point(world, target, target_anchor),
        );
    }

    fn assert_current_diagnostic(
        world: &World,
        source: Entity,
        target: Entity,
        reason: AnchorResolveSkip,
    ) {
        assert!(diagnostics(world).current().any(|entry| {
            entry.source == source && entry.target == target && entry.reason == reason
        }));
    }

    fn diagnostics(world: &World) -> &AttachmentResolveDiagnostics<AnchorResolveSkip> {
        world.resource::<AnchorResolveDiagnostics>()
    }

    fn world_anchor_point(world: &World, entity: Entity, anchor_site: AnchorSite) -> Vec3 {
        let transform = transform(world, entity);
        let frame = geometry(world, entity).frame(anchor_site).copied();
        assert!(frame.is_ok(), "geometry contains requested anchor site");
        let position = frame.unwrap_or_default().position().into_inner();
        transform.translation + transform.rotation * (transform.scale * position)
    }

    fn cached_anchor_point(world: &World, entity: Entity, anchor_site: AnchorSite) -> Vec3 {
        world
            .get::<ResolvedAnchorWorld>(entity)
            .and_then(|anchor_world| anchor_world.points.get(&anchor_site).copied())
            .unwrap_or(Vec3::NAN)
    }

    fn transform(world: &World, entity: Entity) -> Transform {
        world.get::<Transform>(entity).copied().unwrap_or_default()
    }

    fn global_transform(world: &World, entity: Entity) -> GlobalTransform {
        world
            .get::<GlobalTransform>(entity)
            .copied()
            .unwrap_or_default()
    }

    fn geometry(world: &World, entity: Entity) -> ResolvedAnchorGeometry {
        world
            .get::<ResolvedAnchorGeometry>(entity)
            .cloned()
            .unwrap_or_default()
    }

    fn valid<T: Default, E>(result: Result<T, E>) -> T {
        assert!(result.is_ok(), "test setup must be valid");
        result.unwrap_or_default()
    }

    fn assert_close_vec3(actual: Vec3, expected: Vec3) {
        assert!(
            (actual - expected).length() <= ASSERT_EPSILON,
            "actual {actual:?}, expected {expected:?}",
        );
    }

    fn assert_close_quat(actual: Quat, expected: Quat) {
        assert!(
            actual.dot(expected).abs() >= 1.0 - ASSERT_EPSILON,
            "actual {actual:?}, expected {expected:?}",
        );
    }
}
