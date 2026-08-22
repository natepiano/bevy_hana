//! Dependency ordering for pre-classified attachment relations.

use std::collections::VecDeque;
use std::fmt::Debug;
use std::hash::Hash;

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::Resource;
use bevy_platform::collections::HashMap;
use bevy_platform::collections::HashSet;

use crate::AnchoredTo;

/// Attachment edge after consumer-owned validation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AttachmentResolveCandidate<R> {
    /// Edge can be resolved by the consumer's coordinate-space adapter.
    Active {
        /// Entity whose transform or pose will be written.
        source:     Entity,
        /// Entity that provides the target anchor.
        target:     Entity,
        /// Stored relationship payload for the source entity.
        attachment: AnchoredTo,
    },
    /// Edge is owned by this resolver but invalid for this frame.
    Skipped {
        /// Entity that cannot be resolved this frame.
        source: Entity,
        /// Entity that the source attempted to target.
        target: Entity,
        /// Consumer-specific skip reason.
        reason: R,
    },
}

/// Consumer-provided skip reasons used by the dependency resolver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttachmentResolveReasons<R> {
    /// Reason recorded when a source depends on an already-skipped target.
    pub blocked_by_skipped_dependency: R,
    /// Reason recorded for an entity that participates in an attachment cycle.
    pub cycle:                         R,
    /// Reason recorded for an entity blocked by an attachment cycle.
    pub blocked_by_cycle:              R,
}

/// Coordinate-space-specific work requested by the attachment resolver.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AttachmentResolveAction {
    /// Place `source` from `target` using `attachment`.
    Place {
        /// Entity whose transform or pose should be written.
        source:     Entity,
        /// Entity that provides the target anchor.
        target:     Entity,
        /// Stored relationship payload for the source entity.
        attachment: AnchoredTo,
    },
    /// Restore or keep the source entity's fallback placement.
    Fallback {
        /// Entity that should use consumer-owned fallback placement.
        source: Entity,
    },
}

/// Reusable, opaque allocation storage for [`resolve_attachments_with_scratch`].
///
/// Keep one value per resolver owner, normally as Bevy [`Local`] system state.
/// [`resolve_attachments_with_scratch`] drains the caller-owned candidate
/// vector, rebuilds its dependency graph from those current candidates, and
/// clears every entity relationship and traversal state before returning. The
/// scratch value therefore preserves only allocation capacity between calls;
/// it never retains topology, diagnostics, or reconciliation state.
///
/// The scratch does not include candidates because candidate classification is
/// owned by each coordinate-space consumer. A consumer that resolves every
/// frame should keep both this value and its candidate vector in its local
/// resolver state.
///
/// [`Local`]: bevy_ecs::system::Local
#[derive(Default)]
pub struct AttachmentResolverScratch {
    graph:         AttachmentGraph,
    states:        HashMap<Entity, AttachmentResolveState>,
    queue:         VecDeque<Entity>,
    unresolved:    HashSet<Entity>,
    cycle_members: HashSet<Entity>,
    cycle_path:    Vec<Entity>,
    path_indices:  HashMap<Entity, usize>,
}

impl AttachmentResolverScratch {
    fn clear(&mut self) {
        self.graph.clear();
        self.states.clear();
        self.queue.clear();
        self.unresolved.clear();
        self.cycle_members.clear();
        self.cycle_path.clear();
        self.path_indices.clear();
    }
}

/// Resolves active candidates in dependency order and reports skipped edges.
///
/// Consumers classify candidates as [`AttachmentResolveCandidate::Active`] or
/// [`AttachmentResolveCandidate::Skipped`] before calling this function. The
/// resolver owns dependency ordering, fallback dispatch, cycle reporting, and
/// diagnostics accumulation.
pub fn resolve_attachments<R, F>(
    candidates: Vec<AttachmentResolveCandidate<R>>,
    reasons: AttachmentResolveReasons<R>,
    diagnostics: &mut AttachmentResolveDiagnostics<R>,
    handle: F,
) where
    R: Copy + Debug + Eq + Hash + Send + Sync + 'static,
    F: FnMut(AttachmentResolveAction) -> Result<(), R>,
{
    diagnostics.begin_frame();

    let mut scratch = AttachmentResolverScratch::default();
    resolve_attachment_candidates(candidates, reasons, diagnostics, &mut scratch, handle);
}

/// Resolves current candidates while preserving reusable scratch allocations.
///
/// This is the reusable counterpart to [`resolve_attachments`]. It drains
/// `candidates` after classifying them for the current invocation, then clears
/// `scratch` of all entity data before returning. Refill the same vector on a
/// later call with newly read ECS state; no candidate, dependency, cycle, or
/// fallback state from an earlier call participates in that resolution.
///
/// `diagnostics` remains caller-owned so its bounded history has the same
/// behavior as the one-shot API.
pub fn resolve_attachments_with_scratch<R, F>(
    candidates: &mut Vec<AttachmentResolveCandidate<R>>,
    reasons: AttachmentResolveReasons<R>,
    diagnostics: &mut AttachmentResolveDiagnostics<R>,
    scratch: &mut AttachmentResolverScratch,
    handle: F,
) where
    R: Copy + Debug + Eq + Hash + Send + Sync + 'static,
    F: FnMut(AttachmentResolveAction) -> Result<(), R>,
{
    diagnostics.begin_frame();
    resolve_attachment_candidates(candidates.drain(..), reasons, diagnostics, scratch, handle);
}

fn resolve_attachment_candidates<R, F>(
    candidates: impl IntoIterator<Item = AttachmentResolveCandidate<R>>,
    reasons: AttachmentResolveReasons<R>,
    diagnostics: &mut AttachmentResolveDiagnostics<R>,
    scratch: &mut AttachmentResolverScratch,
    mut handle: F,
) where
    R: Copy + Debug + Eq + Hash + Send + Sync + 'static,
    F: FnMut(AttachmentResolveAction) -> Result<(), R>,
{
    scratch.clear();
    for candidate in candidates {
        match candidate {
            AttachmentResolveCandidate::Active {
                source,
                target,
                attachment,
            } => scratch.graph.add(source, target, attachment),
            AttachmentResolveCandidate::Skipped {
                source,
                target,
                reason,
            } => {
                scratch
                    .states
                    .insert(source, AttachmentResolveState::Skipped);
                diagnostics.record(source, target, reason);
                apply_fallback(source, &mut handle);
            },
        }
    }

    let AttachmentResolverScratch {
        graph,
        states,
        queue,
        unresolved,
        cycle_members,
        cycle_path,
        path_indices,
    } = scratch;
    graph.resolve(states, reasons, diagnostics, &mut handle, queue);
    graph.mark_unresolved_cycles(
        states,
        reasons,
        diagnostics,
        &mut handle,
        unresolved,
        cycle_members,
        cycle_path,
        path_indices,
    );
    scratch.clear();
}

#[derive(Default)]
struct AttachmentAdjacency {
    lists_by_target: HashMap<Entity, usize>,
    children:        Vec<Vec<Entity>>,
    assigned_lists:  usize,
}

impl AttachmentAdjacency {
    fn clear(&mut self) {
        self.lists_by_target.clear();
        for children in &mut self.children[..self.assigned_lists] {
            children.clear();
        }
        self.assigned_lists = 0;
    }

    fn add(&mut self, target: Entity, source: Entity) {
        let list_index = self
            .lists_by_target
            .get(&target)
            .copied()
            .unwrap_or_else(|| {
                let list_index = self.assigned_lists;
                self.assigned_lists += 1;
                if list_index == self.children.len() {
                    self.children.push(Vec::new());
                }
                self.lists_by_target.insert(target, list_index);
                list_index
            });
        self.children[list_index].push(source);
    }

    fn children(&self, target: Entity) -> &[Entity] {
        self.lists_by_target
            .get(&target)
            .map_or(&[], |&list_index| self.children[list_index].as_slice())
    }
}

/// Attachment dependency graph built from active candidates.
#[derive(Default)]
struct AttachmentGraph {
    adjacency:   AttachmentAdjacency,
    attachments: HashMap<Entity, AnchoredTo>,
    indegree:    HashMap<Entity, usize>,
    target_of:   HashMap<Entity, Entity>,
}

impl AttachmentGraph {
    fn clear(&mut self) {
        self.adjacency.clear();
        self.attachments.clear();
        self.indegree.clear();
        self.target_of.clear();
    }

    fn add(&mut self, source: Entity, target: Entity, attachment: AnchoredTo) {
        self.adjacency.add(target, source);
        self.attachments.insert(source, attachment);
        self.target_of.insert(source, target);
        *self.indegree.entry(source).or_default() += 1;
        self.indegree.entry(target).or_default();
    }

    fn resolve<R, F>(
        &mut self,
        states: &mut HashMap<Entity, AttachmentResolveState>,
        reasons: AttachmentResolveReasons<R>,
        diagnostics: &mut AttachmentResolveDiagnostics<R>,
        handle: &mut F,
        queue: &mut VecDeque<Entity>,
    ) where
        R: Copy + Debug + Eq + Hash + Send + Sync + 'static,
        F: FnMut(AttachmentResolveAction) -> Result<(), R>,
    {
        for (&entity, &indegree) in &self.indegree {
            if indegree == 0 {
                queue.push_back(entity);
            }
        }

        while let Some(entity) = queue.pop_front() {
            let state = states
                .get(&entity)
                .copied()
                .unwrap_or(AttachmentResolveState::Configured);
            self.resolve_children(entity, state, states, reasons, diagnostics, handle, queue);
        }
    }

    fn resolve_children<R, F>(
        &mut self,
        target: Entity,
        target_state: AttachmentResolveState,
        states: &mut HashMap<Entity, AttachmentResolveState>,
        reasons: AttachmentResolveReasons<R>,
        diagnostics: &mut AttachmentResolveDiagnostics<R>,
        handle: &mut F,
        queue: &mut VecDeque<Entity>,
    ) where
        R: Copy + Debug + Eq + Hash + Send + Sync + 'static,
        F: FnMut(AttachmentResolveAction) -> Result<(), R>,
    {
        let children = self.adjacency.children(target);
        for &child in children {
            match target_state {
                AttachmentResolveState::Skipped => {
                    let reason = reasons.blocked_by_skipped_dependency;
                    states.insert(child, AttachmentResolveState::Skipped);
                    diagnostics.record(child, target, reason);
                    apply_fallback(child, handle);
                },
                AttachmentResolveState::Configured | AttachmentResolveState::Resolved => {
                    Self::resolve_child_position(
                        &self.attachments,
                        child,
                        target,
                        states,
                        diagnostics,
                        handle,
                    );
                },
            }
            if let Some(indegree) = self.indegree.get_mut(&child) {
                *indegree = indegree.saturating_sub(1);
                if *indegree == 0 {
                    queue.push_back(child);
                }
            }
        }
    }

    fn resolve_child_position<R, F>(
        attachments: &HashMap<Entity, AnchoredTo>,
        child: Entity,
        target: Entity,
        states: &mut HashMap<Entity, AttachmentResolveState>,
        diagnostics: &mut AttachmentResolveDiagnostics<R>,
        handle: &mut F,
    ) where
        R: Copy + Debug + Eq + Hash + Send + Sync + 'static,
        F: FnMut(AttachmentResolveAction) -> Result<(), R>,
    {
        let Some(attachment) = attachments.get(&child).copied() else {
            return;
        };
        match handle(AttachmentResolveAction::Place {
            source: child,
            target,
            attachment,
        }) {
            Ok(()) => {
                states.insert(child, AttachmentResolveState::Resolved);
            },
            Err(reason) => {
                states.insert(child, AttachmentResolveState::Skipped);
                diagnostics.record(child, target, reason);
                apply_fallback(child, handle);
            },
        }
    }

    fn mark_unresolved_cycles<R, F>(
        &self,
        states: &mut HashMap<Entity, AttachmentResolveState>,
        reasons: AttachmentResolveReasons<R>,
        diagnostics: &mut AttachmentResolveDiagnostics<R>,
        handle: &mut F,
        unresolved: &mut HashSet<Entity>,
        cycle_members: &mut HashSet<Entity>,
        cycle_path: &mut Vec<Entity>,
        path_indices: &mut HashMap<Entity, usize>,
    ) where
        R: Copy + Debug + Eq + Hash + Send + Sync + 'static,
        F: FnMut(AttachmentResolveAction) -> Result<(), R>,
    {
        unresolved.extend(
            self.indegree
                .iter()
                .filter_map(|(&entity, &indegree)| (indegree > 0).then_some(entity)),
        );
        collect_cycle_members(
            unresolved,
            &self.target_of,
            cycle_members,
            cycle_path,
            path_indices,
        );
        for &entity in unresolved.iter() {
            let reason = if cycle_members.contains(&entity) {
                reasons.cycle
            } else {
                reasons.blocked_by_cycle
            };
            let target = self.target_of.get(&entity).copied().unwrap_or(entity);
            states.insert(entity, AttachmentResolveState::Skipped);
            diagnostics.record(entity, target, reason);
            apply_fallback(entity, handle);
        }
    }
}

fn collect_cycle_members(
    unresolved: &HashSet<Entity>,
    target_of: &HashMap<Entity, Entity>,
    cycle_members: &mut HashSet<Entity>,
    path: &mut Vec<Entity>,
    path_indices: &mut HashMap<Entity, usize>,
) {
    for &start in unresolved {
        path.clear();
        path_indices.clear();
        let mut current = start;
        loop {
            if let Some(&index) = path_indices.get(&current) {
                for &entity in &path[index..] {
                    cycle_members.insert(entity);
                }
                break;
            }
            if !unresolved.contains(&current) {
                break;
            }
            path_indices.insert(current, path.len());
            path.push(current);
            let Some(&target) = target_of.get(&current) else {
                break;
            };
            current = target;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttachmentResolveState {
    Configured,
    Resolved,
    Skipped,
}

fn apply_fallback<R>(
    source: Entity,
    handle: &mut impl FnMut(AttachmentResolveAction) -> Result<(), R>,
) {
    let _ = handle(AttachmentResolveAction::Fallback { source });
}

/// Bounded history of attachment resolution failures.
#[derive(Resource, Debug)]
pub struct AttachmentResolveDiagnostics<R: Send + Sync + 'static> {
    current_frame: u64,
    entries:       VecDeque<AttachmentResolveDiagnostic<R>>,
    capacity:      usize,
}

impl<R: Send + Sync + 'static> AttachmentResolveDiagnostics<R> {
    /// Default number of diagnostic entries retained in insertion order.
    pub const DEFAULT_CAPACITY: usize = 128;

    const fn begin_frame(&mut self) { self.current_frame = self.current_frame.saturating_add(1); }

    fn record(&mut self, source: Entity, target: Entity, reason: R)
    where
        R: Copy + Debug + Eq,
    {
        if let Some(entry) = self.entries.iter_mut().find(|entry| {
            entry.source == source && entry.target == target && entry.reason == reason
        }) {
            entry.last_seen_frame = self.current_frame;
            entry.count = entry.count.saturating_add(1);
            tracing::warn!(
                source = ?source,
                target = ?target,
                reason = ?reason,
                count = entry.count,
                "attachment skip repeated"
            );
            return;
        }

        self.entries.push_back(AttachmentResolveDiagnostic {
            source,
            target,
            reason,
            first_seen_frame: self.current_frame,
            last_seen_frame: self.current_frame,
            count: 1,
        });
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }

    /// Iterates over every retained diagnostic entry.
    pub fn entries(&self) -> impl Iterator<Item = &AttachmentResolveDiagnostic<R>> {
        self.entries.iter()
    }

    /// Iterates over diagnostic entries recorded in the current resolve frame.
    pub fn current(&self) -> impl Iterator<Item = &AttachmentResolveDiagnostic<R>> {
        self.entries
            .iter()
            .filter(|entry| entry.last_seen_frame == self.current_frame)
    }

    /// Number of retained diagnostic entries.
    #[must_use]
    pub fn len(&self) -> usize { self.entries.len() }

    /// Whether no diagnostic entries are retained.
    #[must_use]
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
}

impl<R: Send + Sync + 'static> Default for AttachmentResolveDiagnostics<R> {
    fn default() -> Self {
        Self {
            current_frame: 0,
            entries:       VecDeque::new(),
            capacity:      Self::DEFAULT_CAPACITY,
        }
    }
}

/// One diagnostic entry for a skipped attachment edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttachmentResolveDiagnostic<R> {
    /// Entity that could not be resolved.
    pub source:           Entity,
    /// Entity that `source` attempted to target.
    pub target:           Entity,
    /// Consumer-specific skip reason.
    pub reason:           R,
    /// First resolve frame that recorded this source, target, and reason.
    pub first_seen_frame: u64,
    /// Most recent resolve frame that recorded this source, target, and reason.
    pub last_seen_frame:  u64,
    /// Number of times this source, target, and reason has been recorded.
    pub count:            u32,
}

#[cfg(test)]
mod tests {
    use bevy_ecs::entity::Entity;

    use super::AttachmentResolveAction;
    use super::AttachmentResolveCandidate;
    use super::AttachmentResolveDiagnostic;
    use super::AttachmentResolveDiagnostics;
    use super::AttachmentResolveReasons;
    use super::AttachmentResolverScratch;
    use super::resolve_attachments;
    use super::resolve_attachments_with_scratch;
    use crate::AnchorSite;
    use crate::AnchoredTo;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestSkip {
        BlockedByCycle,
        BlockedBySkippedDependency,
        Cycle,
        MissingTarget,
        PlacementFailed,
        SourceInvalid,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct ScratchStorageObservation {
        adjacency_table:  usize,
        attachments:      usize,
        child_list_table: Allocation,
        child_lists:      Vec<Allocation>,
        cycle_members:    usize,
        cycle_path:       Allocation,
        indegrees:        usize,
        path_indices:     usize,
        queue:            usize,
        states:           usize,
        targets:          usize,
        unresolved:       usize,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct Allocation {
        address:  usize,
        capacity: usize,
    }

    fn entity(index: u64) -> Entity { Entity::from_bits(index) }

    fn attachment(target: Entity) -> AnchoredTo {
        AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center)
    }

    const fn test_reasons() -> AttachmentResolveReasons<TestSkip> {
        AttachmentResolveReasons {
            blocked_by_skipped_dependency: TestSkip::BlockedBySkippedDependency,
            cycle:                         TestSkip::Cycle,
            blocked_by_cycle:              TestSkip::BlockedByCycle,
        }
    }

    fn active(source: Entity, target: Entity) -> AttachmentResolveCandidate<TestSkip> {
        AttachmentResolveCandidate::Active {
            source,
            target,
            attachment: attachment(target),
        }
    }

    fn observe_storage(scratch: &AttachmentResolverScratch) -> ScratchStorageObservation {
        ScratchStorageObservation {
            adjacency_table:  scratch.graph.adjacency.lists_by_target.capacity(),
            attachments:      scratch.graph.attachments.capacity(),
            child_list_table: allocation(&scratch.graph.adjacency.children),
            child_lists:      scratch
                .graph
                .adjacency
                .children
                .iter()
                .map(allocation)
                .collect(),
            cycle_members:    scratch.cycle_members.capacity(),
            cycle_path:       allocation(&scratch.cycle_path),
            indegrees:        scratch.graph.indegree.capacity(),
            path_indices:     scratch.path_indices.capacity(),
            queue:            scratch.queue.capacity(),
            states:           scratch.states.capacity(),
            targets:          scratch.graph.target_of.capacity(),
            unresolved:       scratch.unresolved.capacity(),
        }
    }

    fn allocation<T>(values: &Vec<T>) -> Allocation {
        Allocation {
            address:  values.as_ptr().addr(),
            capacity: values.capacity(),
        }
    }

    #[test]
    fn chain_resolves_parent_before_child_in_one_pass() {
        let root = entity(1);
        let middle = entity(2);
        let leaf = entity(3);
        let mut diagnostics = AttachmentResolveDiagnostics::default();
        let mut actions = Vec::new();

        resolve_attachments(
            vec![
                AttachmentResolveCandidate::Active {
                    source:     leaf,
                    target:     middle,
                    attachment: attachment(middle),
                },
                AttachmentResolveCandidate::Active {
                    source:     middle,
                    target:     root,
                    attachment: attachment(root),
                },
            ],
            test_reasons(),
            &mut diagnostics,
            |action| {
                actions.push(action);
                Ok(())
            },
        );

        assert_eq!(
            actions,
            vec![
                AttachmentResolveAction::Place {
                    source:     middle,
                    target:     root,
                    attachment: attachment(root),
                },
                AttachmentResolveAction::Place {
                    source:     leaf,
                    target:     middle,
                    attachment: attachment(middle),
                },
            ]
        );
        assert!(diagnostics.current().next().is_none());
    }

    #[test]
    fn reusable_scratch_preserves_temporary_storage_for_changed_inputs() {
        let cycle_a = entity(1);
        let cycle_b = entity(2);
        let blocked = entity(3);
        let root = entity(4);
        let leaf = entity(5);
        let missing_target = entity(6);
        let mut candidates = vec![
            active(cycle_a, cycle_b),
            active(cycle_b, cycle_a),
            active(blocked, cycle_a),
            active(leaf, root),
        ];
        let mut diagnostics = AttachmentResolveDiagnostics::default();
        let mut scratch = AttachmentResolverScratch::default();
        let mut first_actions = Vec::new();

        resolve_attachments_with_scratch(
            &mut candidates,
            test_reasons(),
            &mut diagnostics,
            &mut scratch,
            |action| {
                first_actions.push(action);
                Ok(())
            },
        );
        let first_storage = observe_storage(&scratch);
        let candidate_storage = allocation(&candidates);

        assert!(candidates.is_empty());
        assert!(first_storage.queue > 0);
        assert!(first_storage.unresolved > 0);
        assert!(first_storage.cycle_members > 0);
        assert!(first_storage.cycle_path.capacity > 0);
        assert!(first_storage.path_indices > 0);
        assert!(first_actions.contains(&AttachmentResolveAction::Place {
            source:     leaf,
            target:     root,
            attachment: attachment(root),
        }));
        assert!(first_actions.contains(&AttachmentResolveAction::Fallback { source: blocked }));

        candidates.extend([
            active(cycle_a, cycle_b),
            active(cycle_b, cycle_a),
            AttachmentResolveCandidate::Skipped {
                source: blocked,
                target: missing_target,
                reason: TestSkip::MissingTarget,
            },
            active(leaf, root),
        ]);
        let mut second_actions = Vec::new();
        resolve_attachments_with_scratch(
            &mut candidates,
            test_reasons(),
            &mut diagnostics,
            &mut scratch,
            |action| {
                second_actions.push(action);
                Ok(())
            },
        );

        assert_eq!(observe_storage(&scratch), first_storage);
        assert_eq!(allocation(&candidates), candidate_storage);
        assert!(candidates.is_empty());
        assert!(second_actions.contains(&AttachmentResolveAction::Fallback { source: blocked }));
        assert!(diagnostics.current().any(|entry| {
            entry.source == blocked
                && entry.target == missing_target
                && entry.reason == TestSkip::MissingTarget
        }));
        assert!(
            !diagnostics.current().any(|entry| {
                entry.source == blocked && entry.reason == TestSkip::BlockedByCycle
            })
        );
    }

    #[test]
    fn skipped_candidate_routes_to_fallback_and_records_reason() {
        let source = entity(1);
        let target = entity(2);
        let mut diagnostics = AttachmentResolveDiagnostics::default();
        let mut actions = Vec::new();

        resolve_attachments(
            vec![AttachmentResolveCandidate::Skipped {
                source,
                target,
                reason: TestSkip::SourceInvalid,
            }],
            test_reasons(),
            &mut diagnostics,
            |action| {
                actions.push(action);
                Ok(())
            },
        );

        assert_eq!(actions, vec![AttachmentResolveAction::Fallback { source }]);
        assert_eq!(
            diagnostics.current().copied().collect::<Vec<_>>(),
            vec![AttachmentResolveDiagnostic {
                source,
                target,
                reason: TestSkip::SourceInvalid,
                first_seen_frame: 1,
                last_seen_frame: 1,
                count: 1,
            }]
        );
    }

    #[test]
    fn diagnostics_accumulate_across_frames() {
        let source = entity(1);
        let target = entity(2);
        let mut diagnostics = AttachmentResolveDiagnostics {
            capacity: 2,
            ..Default::default()
        };

        resolve_attachments(
            vec![AttachmentResolveCandidate::Skipped {
                source,
                target,
                reason: TestSkip::PlacementFailed,
            }],
            test_reasons(),
            &mut diagnostics,
            |_| Ok(()),
        );
        resolve_attachments(
            vec![AttachmentResolveCandidate::Skipped {
                source,
                target,
                reason: TestSkip::PlacementFailed,
            }],
            test_reasons(),
            &mut diagnostics,
            |_| Ok(()),
        );
        resolve_attachments(
            Vec::<AttachmentResolveCandidate<TestSkip>>::new(),
            test_reasons(),
            &mut diagnostics,
            |_| Ok(()),
        );

        assert_eq!(
            diagnostics.entries().copied().collect::<Vec<_>>(),
            vec![AttachmentResolveDiagnostic {
                source,
                target,
                reason: TestSkip::PlacementFailed,
                first_seen_frame: 1,
                last_seen_frame: 2,
                count: 2,
            }]
        );
        assert!(diagnostics.current().next().is_none());
    }

    #[test]
    fn missing_target_skip_uses_fallback_path() {
        let source = entity(1);
        let target = entity(2);
        let mut diagnostics = AttachmentResolveDiagnostics::default();
        let mut actions = Vec::new();

        resolve_attachments(
            vec![AttachmentResolveCandidate::Skipped {
                source,
                target,
                reason: TestSkip::MissingTarget,
            }],
            test_reasons(),
            &mut diagnostics,
            |action| {
                actions.push(action);
                Ok(())
            },
        );

        assert_eq!(actions, vec![AttachmentResolveAction::Fallback { source }]);
        assert_eq!(
            diagnostics.current().copied().collect::<Vec<_>>(),
            vec![AttachmentResolveDiagnostic {
                source,
                target,
                reason: TestSkip::MissingTarget,
                first_seen_frame: 1,
                last_seen_frame: 1,
                count: 1,
            }]
        );
    }
}
