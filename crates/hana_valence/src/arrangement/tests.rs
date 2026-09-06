use core::convert::Infallible;
use core::error::Error;
use core::fmt::Display;
use core::fmt::Formatter;

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use super::ArrangementConnection;
use super::ArrangementConnectionDisplacement;
use super::ArrangementError;
use super::ArrangementMemberEntities;
use super::ArrangementPlan;
use super::ArrangementProvider;
use super::HingeClearance;
use super::Members;
use super::RetainedProviderKnowledge;
use super::materialize;
use crate::AnchorSite;
use crate::AnchoredTo;
use crate::Angle;
use crate::Displacement;
use crate::Edge;
use crate::FoldGroup;
use crate::FoldGroups;
use crate::ProviderCapability;

static_assertions::assert_not_impl_any!(Members:
    Clone,
    bevy_reflect::Reflect,
    bevy_reflect::FromReflect
);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum LogicalMember {
    First,
    Root,
    Second,
    Third,
}

struct NoFoldProvider;

#[derive(Debug)]
struct ProviderFailure;

impl Display for ProviderFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("provider authoring failed")
    }
}

impl Error for ProviderFailure {}

impl ArrangementProvider for NoFoldProvider {
    type Member = LogicalMember;
    type FoldGroupSelection = Infallible;

    fn members(&self) -> impl Iterator<Item = Self::Member> {
        [
            LogicalMember::Root,
            LogicalMember::First,
            LogicalMember::Second,
        ]
        .into_iter()
    }

    fn generate_plan(
        &self,
        members: &ArrangementMemberEntities<Self::Member>,
    ) -> Result<ArrangementPlan<Self::FoldGroupSelection>, ArrangementError> {
        let root = members.entity(&LogicalMember::Root)?;
        let first = members.entity(&LogicalMember::First)?;
        let second = members.entity(&LogicalMember::Second)?;

        ArrangementPlan::try_new(
            members,
            [
                arrangement_connection(first, root),
                arrangement_connection(second, root),
            ],
        )
    }
}

fn arrangement_connection(member_entity: Entity, target_entity: Entity) -> ArrangementConnection {
    arrangement_connection_with_edge(
        member_entity,
        target_entity,
        Edge {
            start: AnchorSite::Vertex(0),
            end:   AnchorSite::Vertex(1),
        },
    )
}

fn arrangement_connection_with_edge(
    member_entity: Entity,
    target_entity: Entity,
    member_edge: Edge,
) -> ArrangementConnection {
    ArrangementConnection {
        member_entity,
        anchored_to: AnchoredTo::new(target_entity, AnchorSite::Vertex(0), AnchorSite::Vertex(1)),
        member_edge,
        base_angle: Angle::default(),
        hinge_clearance: HingeClearance::CENTERED,
    }
}

fn valid<T, E: core::fmt::Debug>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("expected a valid result, got {error:?}"),
    }
}

fn plan_entities(
    world: &mut World,
    members: impl IntoIterator<Item = LogicalMember>,
) -> ArrangementMemberEntities<LogicalMember> {
    valid(ArrangementMemberEntities::try_new(
        members
            .into_iter()
            .map(|member| (member, world.spawn_empty().id())),
    ))
}

fn member_entity(
    members: &ArrangementMemberEntities<LogicalMember>,
    member: LogicalMember,
) -> Entity {
    valid(members.entity(&member))
}

#[test]
fn arrangement_member_entities_preserve_provider_order_and_report_unlisted_members() {
    let mut world = World::new();
    let members = plan_entities(
        &mut world,
        [
            LogicalMember::Second,
            LogicalMember::Root,
            LogicalMember::First,
        ],
    );

    assert_eq!(
        members
            .iter()
            .map(|(member, _)| *member)
            .collect::<Vec<_>>(),
        vec![
            LogicalMember::Second,
            LogicalMember::Root,
            LogicalMember::First,
        ],
    );
    assert_eq!(members.len(), 3);
    assert!(!members.is_empty());
    assert!(matches!(
        members.entity(&LogicalMember::Third),
        Err(ArrangementError::UnlistedMember { member }) if member == "Third",
    ));
}

#[test]
fn library_association_construction_rejects_duplicate_logical_members_and_entities() {
    let mut world = World::new();
    let first = world.spawn_empty().id();
    let second = world.spawn_empty().id();

    assert!(matches!(
        ArrangementMemberEntities::try_new([
            (LogicalMember::Root, first),
            (LogicalMember::Root, second),
        ]),
        Err(ArrangementError::DuplicateLogicalMember { member }) if member == "Root",
    ));
    assert!(matches!(
        ArrangementMemberEntities::try_new([
            (LogicalMember::Root, first),
            (LogicalMember::First, first),
        ]),
        Err(ArrangementError::DuplicateMemberEntity { member_entity }) if member_entity == first,
    ));
}

#[test]
fn empty_member_association_produces_an_empty_typed_plan() {
    let members = valid(ArrangementMemberEntities::<LogicalMember>::try_new([]));
    let plan = valid(ArrangementPlan::<core::convert::Infallible>::try_new(
        &members,
        [],
    ));

    assert!(members.is_empty());
    assert!(plan.connections().is_empty());
}

#[test]
fn plan_accepts_branches_and_repeated_targets_in_logical_member_order() {
    let mut world = World::new();
    let members = plan_entities(
        &mut world,
        [
            LogicalMember::Root,
            LogicalMember::First,
            LogicalMember::Second,
            LogicalMember::Third,
        ],
    );
    let root = member_entity(&members, LogicalMember::Root);
    let first = member_entity(&members, LogicalMember::First);
    let second = member_entity(&members, LogicalMember::Second);
    let third = member_entity(&members, LogicalMember::Third);

    let plan = valid(ArrangementPlan::<core::convert::Infallible>::try_new(
        &members,
        [
            arrangement_connection(third, root),
            arrangement_connection(second, root),
            arrangement_connection(first, root),
        ],
    ));

    assert_eq!(
        plan.connections()
            .iter()
            .map(|connection| connection.member_entity)
            .collect::<Vec<_>>(),
        vec![first, second, third],
    );
}

#[test]
fn plan_accepts_disconnected_forests_with_multiple_roots() {
    let mut world = World::new();
    let members = plan_entities(
        &mut world,
        [
            LogicalMember::Root,
            LogicalMember::First,
            LogicalMember::Second,
            LogicalMember::Third,
        ],
    );
    let root = member_entity(&members, LogicalMember::Root);
    let first = member_entity(&members, LogicalMember::First);
    let second = member_entity(&members, LogicalMember::Second);
    let third = member_entity(&members, LogicalMember::Third);

    let plan = ArrangementPlan::<core::convert::Infallible>::try_new(
        &members,
        [
            arrangement_connection(third, second),
            arrangement_connection(first, root),
        ],
    );

    assert!(plan.is_ok());
}

#[test]
fn plan_accepts_finite_multi_turn_base_angles() {
    let mut world = World::new();
    let members = plan_entities(&mut world, [LogicalMember::Root, LogicalMember::First]);
    let root = member_entity(&members, LogicalMember::Root);
    let first = member_entity(&members, LogicalMember::First);
    let base_angle = valid(Angle::from_radians(core::f32::consts::TAU * 3.5));
    let mut connection = arrangement_connection(first, root);
    connection.base_angle = base_angle;

    let plan = valid(ArrangementPlan::<core::convert::Infallible>::try_new(
        &members,
        [connection],
    ));

    assert_eq!(plan.connections()[0].base_angle, base_angle);
}

#[test]
fn plan_rejects_a_connection_from_an_unlisted_source_without_world_mutation() {
    let mut world = World::new();
    let members = plan_entities(&mut world, [LogicalMember::Root]);
    let target = member_entity(&members, LogicalMember::Root);
    let foreign_source = world.spawn_empty().id();
    let entities_before = world.iter_entities().count();

    let result = ArrangementPlan::<core::convert::Infallible>::try_new(
        &members,
        [arrangement_connection(foreign_source, target)],
    );

    assert!(matches!(
        result,
        Err(ArrangementError::UnlistedConnectionSource { member_entity }) if member_entity == foreign_source,
    ));
    assert_eq!(world.iter_entities().count(), entities_before);
}

#[test]
fn plan_rejects_a_connection_to_an_unlisted_target() {
    let mut world = World::new();
    let members = plan_entities(&mut world, [LogicalMember::Root]);
    let source = member_entity(&members, LogicalMember::Root);
    let foreign_target = world.spawn_empty().id();

    assert!(matches!(
        ArrangementPlan::<core::convert::Infallible>::try_new(
            &members,
            [arrangement_connection(source, foreign_target)],
        ),
        Err(ArrangementError::UnlistedConnectionTarget { target_entity }) if target_entity == foreign_target,
    ));
}

#[test]
fn plan_rejects_self_targets_duplicate_sources_and_physical_cycles() {
    let mut world = World::new();
    let members = plan_entities(
        &mut world,
        [
            LogicalMember::Root,
            LogicalMember::First,
            LogicalMember::Second,
        ],
    );
    let root = member_entity(&members, LogicalMember::Root);
    let first = member_entity(&members, LogicalMember::First);
    let second = member_entity(&members, LogicalMember::Second);

    assert!(matches!(
        ArrangementPlan::<core::convert::Infallible>::try_new(
            &members,
            [arrangement_connection(first, first)],
        ),
        Err(ArrangementError::SelfTarget { member_entity }) if member_entity == first,
    ));
    assert!(matches!(
        ArrangementPlan::<core::convert::Infallible>::try_new(
            &members,
            [
                arrangement_connection(first, root),
                arrangement_connection(first, second),
            ],
        ),
        Err(ArrangementError::DuplicateConnectionSource { member_entity }) if member_entity == first,
    ));
    assert!(matches!(
        ArrangementPlan::<core::convert::Infallible>::try_new(
            &members,
            [
                arrangement_connection(first, second),
                arrangement_connection(second, first),
            ],
        ),
        Err(ArrangementError::PhysicalCycle { member_entity }) if member_entity == first,
    ));
}

#[test]
fn plan_rejects_equal_member_edge_sites_without_geometry() {
    let mut world = World::new();
    let members = plan_entities(&mut world, [LogicalMember::Root, LogicalMember::First]);
    let root = member_entity(&members, LogicalMember::Root);
    let first = member_entity(&members, LogicalMember::First);
    let member_edge = Edge {
        start: AnchorSite::Vertex(4),
        end:   AnchorSite::Vertex(4),
    };

    assert!(matches!(
        ArrangementPlan::<core::convert::Infallible>::try_new(
            &members,
            [arrangement_connection_with_edge(first, root, member_edge)],
        ),
        Err(ArrangementError::EqualMemberEdgeSites {
            member_entity,
            member_edge: rejected_edge,
        }) if member_entity == first && rejected_edge == member_edge,
    ));
}

#[test]
fn plan_accepts_an_unequal_ordered_edge_without_revalidating_geometry() {
    let mut world = World::new();
    let members = plan_entities(&mut world, [LogicalMember::Root, LogicalMember::First]);
    let root = member_entity(&members, LogicalMember::Root);
    let first = member_entity(&members, LogicalMember::First);
    let member_edge = Edge {
        start: AnchorSite::Vertex(17),
        end:   AnchorSite::EdgeMidpoint(29),
    };

    let plan = ArrangementPlan::<core::convert::Infallible>::try_new(
        &members,
        [arrangement_connection_with_edge(first, root, member_edge)],
    );

    assert!(plan.is_ok());
}

#[test]
fn plan_rejects_non_finite_attachment_offset_and_directional_clearance() {
    let mut world = World::new();
    let members = plan_entities(&mut world, [LogicalMember::Root, LogicalMember::First]);
    let root = member_entity(&members, LogicalMember::Root);
    let first = member_entity(&members, LogicalMember::First);
    let mut offset_connection = arrangement_connection(first, root);
    offset_connection.anchored_to =
        offset_connection
            .anchored_to
            .with_offset(Displacement::new(f32::NAN, 0.0, 0.0));
    let mut positive_connection = arrangement_connection(first, root);
    positive_connection.hinge_clearance = HingeClearance::new(
        Displacement::new(f32::INFINITY, 0.0, 0.0),
        Displacement::default(),
    );
    let mut negative_connection = arrangement_connection(first, root);
    negative_connection.hinge_clearance = HingeClearance::new(
        Displacement::default(),
        Displacement::new(0.0, f32::NEG_INFINITY, 0.0),
    );

    assert!(matches!(
        ArrangementPlan::<core::convert::Infallible>::try_new(&members, [offset_connection]),
        Err(ArrangementError::NonFiniteDisplacement {
            member_entity,
            displacement: ArrangementConnectionDisplacement::AttachmentOffset,
        }) if member_entity == first,
    ));
    assert!(matches!(
        ArrangementPlan::<core::convert::Infallible>::try_new(&members, [positive_connection]),
        Err(ArrangementError::NonFiniteDisplacement {
            member_entity,
            displacement: ArrangementConnectionDisplacement::PositiveHingeClearance,
        }) if member_entity == first,
    ));
    assert!(matches!(
        ArrangementPlan::<core::convert::Infallible>::try_new(&members, [negative_connection]),
        Err(ArrangementError::NonFiniteDisplacement {
            member_entity,
            displacement: ArrangementConnectionDisplacement::NegativeHingeClearance,
        }) if member_entity == first,
    ));
}

#[test]
fn provider_plan_retains_its_typed_selection_and_received_association() {
    let mut world = World::new();
    let members = plan_entities(
        &mut world,
        [
            LogicalMember::Root,
            LogicalMember::First,
            LogicalMember::Second,
        ],
    );

    let plan = valid(NoFoldProvider.generate_plan(&members));

    assert_eq!(
        accepts_no_fold_plan(plan),
        [
            member_entity(&members, LogicalMember::First),
            member_entity(&members, LogicalMember::Second),
        ],
    );
}

#[test]
fn provider_errors_preserve_the_downstream_error_source() {
    let error = ArrangementError::provider(ProviderFailure);

    assert_eq!(
        core::error::Error::source(&error).map(std::string::ToString::to_string),
        Some(String::from("provider authoring failed")),
    );
}

fn accepts_no_fold_plan(plan: ArrangementPlan<Infallible>) -> [Entity; 2] {
    [
        plan.connections()[0].member_entity,
        plan.connections()[1].member_entity,
    ]
}

/// Two-member fold alternatives authored by [`FoldingProvider`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum FoldChoice {
    Column,
    Row,
    Unauthored,
}

/// A selection vocabulary from a different provider.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ForeignChoice;

/// Provider capability retained for one selected alternative.
#[derive(Debug, Eq, PartialEq)]
struct MemberSpacing(u8);

impl ProviderCapability for MemberSpacing {}

struct FoldingProvider;

impl ArrangementProvider for FoldingProvider {
    type FoldGroupSelection = FoldChoice;
    type Member = LogicalMember;

    fn members(&self) -> impl Iterator<Item = Self::Member> {
        [
            LogicalMember::Root,
            LogicalMember::First,
            LogicalMember::Second,
        ]
        .into_iter()
    }

    fn generate_plan(
        &self,
        members: &ArrangementMemberEntities<Self::Member>,
    ) -> Result<ArrangementPlan<Self::FoldGroupSelection>, ArrangementError> {
        let root = members.entity(&LogicalMember::Root)?;
        let first = members.entity(&LogicalMember::First)?;
        let second = members.entity(&LogicalMember::Second)?;

        ArrangementPlan::try_new(
            members,
            [
                arrangement_connection(first, root),
                arrangement_connection(second, root),
            ],
        )?
        .with_fold_groups(FoldChoice::Column, folded_together(first, second))?
        .with_fold_groups(FoldChoice::Row, folded_separately(first, second))?
        .with_capability(FoldChoice::Column, MemberSpacing(2))
    }
}

fn folded_together(first: Entity, second: Entity) -> FoldGroups {
    FoldGroups::from(valid(FoldGroup::try_new(first, [second])))
}

fn folded_separately(first: Entity, second: Entity) -> FoldGroups {
    FoldGroups::new(FoldGroup::from(first), [FoldGroup::from(second)])
}

fn folding_arrangement(world: &mut World) -> Entity {
    // Materialization writes baseline hinges, and `Hinge`'s hook refuses a
    // world with no driver; an `App` would install `HingePlugin` for us.
    world.insert_resource(crate::hinge::Pintle::installed());
    let controller = world.spawn_empty().id();
    let members = valid(ArrangementMemberEntities::try_new(
        FoldingProvider
            .members()
            .map(|member| (member, world.spawn_empty().id())),
    ));
    let plan = valid(FoldingProvider.generate_plan(&members));
    {
        let mut commands = world.commands();
        materialize::queue_arrangement_without_scenes(&mut commands, controller, members, plan);
    }
    world.flush();

    controller
}

#[test]
fn one_selection_retains_one_complete_group_alternative_and_its_capability() {
    let mut world = World::new();
    let controller = folding_arrangement(&mut world);
    let retained = valid(RetainedProviderKnowledge::for_arrangement(
        &world, controller,
    ));
    let member_roots = valid(world.get::<Members>(controller).ok_or(()))
        .iter()
        .count();

    assert_eq!(member_roots, 3);
    assert_eq!(
        valid(retained.groups(&FoldChoice::Column)).as_slice().len(),
        1
    );
    assert_eq!(valid(retained.groups(&FoldChoice::Row)).as_slice().len(), 2);
    assert_eq!(
        valid(retained.capability::<FoldChoice, MemberSpacing>(&FoldChoice::Column)),
        &MemberSpacing(2),
    );
}

#[test]
fn a_retained_read_rejects_a_foreign_selection_type_before_any_selection_value() {
    let mut world = World::new();
    let controller = folding_arrangement(&mut world);
    let retained = valid(RetainedProviderKnowledge::for_arrangement(
        &world, controller,
    ));

    assert!(matches!(
        retained.groups(&ForeignChoice),
        Err(ArrangementError::MismatchedFoldGroupSelectionType { expected, found })
            if expected.ends_with("FoldChoice") && found.ends_with("ForeignChoice"),
    ));
    assert!(matches!(
        retained.capability::<ForeignChoice, MemberSpacing>(&ForeignChoice),
        Err(ArrangementError::MismatchedFoldGroupSelectionType { .. }),
    ));
}

#[test]
fn a_retained_read_distinguishes_an_unknown_value_from_a_missing_capability() {
    let mut world = World::new();
    let controller = folding_arrangement(&mut world);
    let retained = valid(RetainedProviderKnowledge::for_arrangement(
        &world, controller,
    ));

    assert!(matches!(
        retained.groups(&FoldChoice::Unauthored),
        Err(ArrangementError::UnknownFoldGroupSelection { selection })
            if selection == "Unauthored",
    ));
    assert!(matches!(
        retained.capability::<FoldChoice, MemberSpacing>(&FoldChoice::Row),
        Err(ArrangementError::MissingCapability { selection, capability })
            if selection == "Row" && capability.ends_with("MemberSpacing"),
    ));
}

#[test]
fn a_retained_read_of_a_plain_entity_names_the_unmaterialized_arrangement() {
    let mut world = World::new();
    let plain = world.spawn_empty().id();

    assert!(matches!(
        RetainedProviderKnowledge::for_arrangement(&world, plain),
        Err(ArrangementError::UnmaterializedArrangement { arrangement }) if arrangement == plain,
    ));
}

#[test]
fn a_plan_rejects_a_repeated_selection_and_a_capability_for_an_unauthored_selection() {
    let mut world = World::new();
    let members = plan_entities(
        &mut world,
        [
            LogicalMember::Root,
            LogicalMember::First,
            LogicalMember::Second,
        ],
    );
    let root = member_entity(&members, LogicalMember::Root);
    let first = member_entity(&members, LogicalMember::First);
    let second = member_entity(&members, LogicalMember::Second);
    let plan = valid(ArrangementPlan::<FoldChoice>::try_new(
        &members,
        [
            arrangement_connection(first, root),
            arrangement_connection(second, root),
        ],
    ));
    let plan = valid(plan.with_fold_groups(FoldChoice::Column, folded_together(first, second)));

    assert!(matches!(
        plan.with_capability(FoldChoice::Row, MemberSpacing(1)),
        Err(ArrangementError::UnknownFoldGroupSelection { selection }) if selection == "Row",
    ));
    let plan = valid(ArrangementPlan::<FoldChoice>::try_new(
        &members,
        [arrangement_connection(first, root)],
    ));
    let plan =
        valid(plan.with_fold_groups(FoldChoice::Column, FoldGroups::from(FoldGroup::from(first))));

    assert!(matches!(
        plan.with_fold_groups(FoldChoice::Column, FoldGroups::from(FoldGroup::from(first))),
        Err(ArrangementError::DuplicateFoldGroupSelection { selection }) if selection == "Column",
    ));
}

#[test]
fn a_plan_rejects_a_second_value_of_one_capability_type_for_one_selection() {
    let mut world = World::new();
    let members = plan_entities(&mut world, [LogicalMember::Root, LogicalMember::First]);
    let root = member_entity(&members, LogicalMember::Root);
    let first = member_entity(&members, LogicalMember::First);
    let plan = valid(ArrangementPlan::<FoldChoice>::try_new(
        &members,
        [arrangement_connection(first, root)],
    ));
    let plan =
        valid(plan.with_fold_groups(FoldChoice::Column, FoldGroups::from(FoldGroup::from(first))));
    let plan = valid(plan.with_capability(FoldChoice::Column, MemberSpacing(1)));

    assert!(matches!(
        plan.with_capability(FoldChoice::Column, MemberSpacing(3)),
        Err(ArrangementError::DuplicateCapability { selection, capability })
            if selection == "Column" && capability.ends_with("MemberSpacing"),
    ));
}

#[test]
fn a_plan_rejects_a_group_member_that_no_connection_attaches() {
    let mut world = World::new();
    let members = plan_entities(&mut world, [LogicalMember::Root, LogicalMember::First]);
    let root = member_entity(&members, LogicalMember::Root);
    let first = member_entity(&members, LogicalMember::First);
    let plan = valid(ArrangementPlan::<FoldChoice>::try_new(
        &members,
        [arrangement_connection(first, root)],
    ));

    assert!(matches!(
        plan.with_fold_groups(FoldChoice::Column, FoldGroups::from(FoldGroup::from(root))),
        Err(ArrangementError::ForeignFoldGroupMember { member_entity }) if member_entity == root,
    ));
}

#[test]
fn overlapping_groups_and_repeated_members_across_groups_are_valid() {
    let mut world = World::new();
    let members = plan_entities(
        &mut world,
        [
            LogicalMember::Root,
            LogicalMember::First,
            LogicalMember::Second,
        ],
    );
    let root = member_entity(&members, LogicalMember::Root);
    let first = member_entity(&members, LogicalMember::First);
    let second = member_entity(&members, LogicalMember::Second);
    let plan = valid(ArrangementPlan::<FoldChoice>::try_new(
        &members,
        [
            arrangement_connection(first, root),
            arrangement_connection(second, root),
        ],
    ));
    let overlapping = FoldGroups::new(
        valid(FoldGroup::try_new(first, [second])),
        [valid(FoldGroup::try_new(second, [first]))],
    );

    assert!(
        plan.with_fold_groups(FoldChoice::Column, overlapping)
            .is_ok()
    );
}
