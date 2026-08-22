use core::any::TypeId;
use core::fmt::Debug;
use core::hash::Hash;

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::Commands;
use bevy_ecs::prelude::Component;
use bevy_ecs::world::World;
use bevy_scene::EntityCommandsSceneExt;
use bevy_scene::Scene;

use super::Arrangement;
use super::ArrangementConnection;
use super::ArrangementError;
use super::ArrangementMemberEntities;
use super::ArrangementPlan;
use super::Member;
use super::Members;
use super::PlannedFoldSequence;
use super::capabilities::RetainedFoldGroupAlternatives;
use crate::Hinge;

/// Connection calibration retained privately for later arrangement phases.
///
/// This component deliberately has no public readers. It preserves the plan's
/// validated connection values after the transient typed association and plan
/// are dropped, without exposing a second mutable arrangement representation.
#[derive(Component)]
pub(super) struct RetainedArrangementConnections(Vec<ArrangementConnection>);

impl RetainedArrangementConnections {
    pub(super) const fn connection_count(&self) -> usize { self.0.len() }

    /// Returns every connection this arrangement retained, in plan order.
    pub(super) fn connections(&self) -> &[ArrangementConnection] { &self.0 }
}

/// Reads the retained connections of a materialized arrangement.
///
/// [`RetainedArrangementConnections`] is the sole connection authority after
/// materialization, so a fold recipe reads its attachments and clearances from
/// here rather than reconstructing them from the ECS relationship graph.
///
/// # Errors
///
/// Returns [`ArrangementError::UnmaterializedArrangement`] when `arrangement`
/// is not a materialized arrangement controller.
pub(crate) fn retained_connections(
    world: &World,
    arrangement: Entity,
) -> Result<&[ArrangementConnection], ArrangementError> {
    world
        .get::<RetainedArrangementConnections>(arrangement)
        .map_or(
            Err(ArrangementError::UnmaterializedArrangement { arrangement }),
            |retained| Ok(retained.connections()),
        )
}

pub(super) fn queue_arrangement<M, Selection, S>(
    commands: &mut Commands<'_, '_>,
    controller: Entity,
    members: ArrangementMemberEntities<M>,
    plan: ArrangementPlan<Selection>,
    scenes: impl IntoIterator<Item = S>,
) where
    M: Eq + Hash + Debug,
    Selection: Eq + Hash + Debug + Send + Sync + 'static,
    S: Scene,
{
    let member_entities = members.iter().map(|(_, entity)| entity).collect::<Vec<_>>();
    queue_components(
        commands,
        controller,
        member_entities.iter().copied(),
        plan,
        MemberWrites::Reserved,
    );
    for (member, scene) in member_entities.into_iter().zip(scenes) {
        commands.entity(member).queue_apply_scene(scene);
    }
}

pub(super) fn queue_arrangement_without_scenes<M, Selection>(
    commands: &mut Commands<'_, '_>,
    controller: Entity,
    members: ArrangementMemberEntities<M>,
    plan: ArrangementPlan<Selection>,
) where
    M: Eq + Hash + Debug,
    Selection: Eq + Hash + Debug + Send + Sync + 'static,
{
    queue_components(
        commands,
        controller,
        members.iter().map(|(_, entity)| entity),
        plan,
        MemberWrites::BestEffort,
    );
}

/// Whether materialization owns the member entities or only borrows them.
///
/// Reserved roots are created by this command and therefore retain ordinary
/// deferred-command failures. Bound roots remain application-owned, so a
/// disappearance after validation is best effort at application time.
#[derive(Clone, Copy)]
enum MemberWrites {
    Reserved,
    BestEffort,
}

fn queue_components<Selection>(
    commands: &mut Commands<'_, '_>,
    controller: Entity,
    members: impl IntoIterator<Item = Entity>,
    plan: ArrangementPlan<Selection>,
    member_writes: MemberWrites,
) where
    Selection: Eq + Hash + Debug + Send + Sync + 'static,
{
    let (connections, alternatives, fold_sequence) = plan.into_parts();
    let alternative_count = alternatives.len();
    let retained_connections = RetainedArrangementConnections(connections.clone());
    let retained_alternatives = RetainedFoldGroupAlternatives::erase(alternatives);
    debug_assert_eq!(retained_connections.connection_count(), connections.len());
    debug_assert_eq!(
        retained_alternatives.selection_type_id(),
        TypeId::of::<Selection>()
    );
    debug_assert_eq!(retained_alternatives.alternative_count(), alternative_count);
    commands.entity(controller).insert((
        Arrangement,
        Members::default(),
        retained_connections,
        retained_alternatives,
    ));
    match fold_sequence {
        PlannedFoldSequence::Unauthored => {},
        PlannedFoldSequence::Authored(fold_sequence) => {
            commands.entity(controller).insert(fold_sequence);
        },
    }
    for member in members {
        let mut member = commands.entity(member);
        match member_writes {
            MemberWrites::Reserved => {
                member.insert(Member::new(controller));
            },
            MemberWrites::BestEffort => {
                member.try_insert(Member::new(controller));
            },
        }
    }
    for connection in connections {
        let hinge = Hinge::resting(connection.member_edge, connection.base_angle);
        let mut member = commands.entity(connection.member_entity);
        match member_writes {
            MemberWrites::Reserved => {
                member.insert((connection.anchored_to, hinge));
            },
            MemberWrites::BestEffort => {
                member.try_insert((connection.anchored_to, hinge));
            },
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use bevy_ecs::world::World;

    use super::RetainedArrangementConnections;
    use super::RetainedFoldGroupAlternatives;
    use super::queue_arrangement_without_scenes;
    use crate::AnchorSite;
    use crate::AnchoredTo;
    use crate::Angle;
    use crate::ArrangementConnection;
    use crate::ArrangementMemberEntities;
    use crate::ArrangementPlan;
    use crate::Edge;
    use crate::HingeClearance;
    use crate::Member;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum LogicalMember {
        Member,
        Root,
    }

    #[test]
    fn materialization_retains_only_private_connection_data_and_selection_identity() {
        let mut world = World::new();
        let controller = world.spawn_empty().id();
        let root = world.spawn_empty().id();
        let member = world.spawn_empty().id();
        let Ok(members) = ArrangementMemberEntities::try_new([
            (LogicalMember::Root, root),
            (LogicalMember::Member, member),
        ]) else {
            panic!("ArrangementMemberEntities::try_new rejected a root and one member");
        };
        let connection = ArrangementConnection {
            member_entity:   member,
            anchored_to:     AnchoredTo::new(root, AnchorSite::Vertex(0), AnchorSite::Vertex(1)),
            member_edge:     Edge {
                start: AnchorSite::Vertex(0),
                end:   AnchorSite::Vertex(1),
            },
            base_angle:      Angle::default(),
            hinge_clearance: HingeClearance::CENTERED,
        };
        let Ok(plan) = ArrangementPlan::<LogicalMember>::try_new(&members, [connection]) else {
            panic!("ArrangementPlan::try_new rejected a single root-to-member connection");
        };
        let Ok(plan) = plan.with_fold_groups(
            LogicalMember::Member,
            crate::FoldGroups::from(crate::FoldGroup::from(member)),
        ) else {
            panic!("with_fold_groups rejected a one-member fold group");
        };

        {
            let mut commands = world.commands();
            queue_arrangement_without_scenes(&mut commands, controller, members, plan);
        }
        world.flush();

        assert_eq!(
            world
                .get::<RetainedArrangementConnections>(controller)
                .map(RetainedArrangementConnections::connection_count),
            Some(1),
        );
        assert_eq!(
            world
                .get::<RetainedFoldGroupAlternatives>(controller)
                .map(RetainedFoldGroupAlternatives::selection_type_id),
            Some(core::any::TypeId::of::<LogicalMember>()),
        );
        assert_eq!(
            world
                .get::<RetainedFoldGroupAlternatives>(controller)
                .map(RetainedFoldGroupAlternatives::alternative_count),
            Some(1),
        );
        assert_eq!(
            world
                .get::<Member>(member)
                .map(|relation| relation.arrangement_entity),
            Some(controller),
        );
        assert_eq!(
            world.get::<AnchoredTo>(member).map(AnchoredTo::target),
            Some(root),
        );
    }
}
