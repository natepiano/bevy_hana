//! Public arrangement construction and relationship coverage.

#![allow(clippy::panic, reason = "tests should panic on unexpected values")]
#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#![allow(
    clippy::unreachable,
    reason = "tests state invariants an earlier assert already guarantees"
)]

use core::cell::Cell;
use core::convert::Infallible;
use core::fmt::Debug;
use core::fmt::Write as _;
use core::hash::Hash;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bevy::app::App;
use bevy::app::PostUpdate;
use bevy::app::TaskPoolPlugin;
use bevy::app::Update;
use bevy::asset::AssetPlugin;
use bevy::ecs::component::Component;
use bevy::ecs::entity::Entity;
use bevy::ecs::event::EntityEvent as _;
use bevy::ecs::observer::On;
use bevy::ecs::prelude::Resource;
use bevy::ecs::query::Changed;
use bevy::ecs::query::With;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::schedule::Schedule;
use bevy::ecs::schedule::SingleThreadedExecutor;
use bevy::ecs::schedule::common_conditions::run_once;
use bevy::ecs::system::Commands;
use bevy::ecs::system::In;
use bevy::ecs::system::Query;
use bevy::ecs::system::Res;
use bevy::ecs::system::ResMut;
use bevy::ecs::system::RunSystemOnce as _;
use bevy::ecs::template::FromTemplate;
use bevy::ecs::world::World;
use bevy::log::tracing::Event;
use bevy::log::tracing::Level;
use bevy::log::tracing::Subscriber;
use bevy::log::tracing::field::Field;
use bevy::log::tracing::field::Visit;
use bevy::log::tracing_subscriber::Layer;
use bevy::log::tracing_subscriber::Registry;
use bevy::log::tracing_subscriber::layer::Context;
use bevy::log::tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use bevy::math::Quat;
use bevy::math::Vec3;
use bevy::math::curve::EaseFunction;
use bevy::scene::ScenePlugin;
use bevy::scene::bsn;
use bevy::time::Time;
use bevy::time::Virtual;
use bevy::transform::TransformPlugin;
use bevy::transform::TransformSystems;
use bevy::transform::components::GlobalTransform;
use bevy::transform::components::Transform;
use hana_valence::Accordion;
use hana_valence::AnchorFrame;
use hana_valence::AnchorPose;
use hana_valence::AnchorResolveDiagnostics;
use hana_valence::AnchorSite;
use hana_valence::AnchorSystems;
use hana_valence::AnchoredHere;
use hana_valence::AnchoredTo;
use hana_valence::Angle;
use hana_valence::Arrangement;
use hana_valence::ArrangementCommandsExt as _;
use hana_valence::ArrangementConnection;
use hana_valence::ArrangementError;
use hana_valence::ArrangementMemberEntities;
use hana_valence::ArrangementPlan;
use hana_valence::ArrangementPlugin;
use hana_valence::ArrangementProvider;
use hana_valence::Coil;
use hana_valence::Displacement;
use hana_valence::Easing;
use hana_valence::EasingCurve;
use hana_valence::EasingInput;
use hana_valence::EasingInterpolation;
use hana_valence::EasingKnot;
use hana_valence::EasingOutput;
use hana_valence::EasingSample;
use hana_valence::EasingSlope;
use hana_valence::EasingSlopes;
use hana_valence::Edge;
use hana_valence::FoldAssignment;
use hana_valence::FoldAuthorError;
use hana_valence::FoldCommands;
use hana_valence::FoldEndpoint;
use hana_valence::FoldEndpointReached;
use hana_valence::FoldEvaluationError;
use hana_valence::FoldEventTiming;
use hana_valence::FoldGroup;
use hana_valence::FoldGroups;
use hana_valence::FoldMemberBegin;
use hana_valence::FoldMemberEnd;
use hana_valence::FoldMemberFraction;
use hana_valence::FoldMemberSample;
use hana_valence::FoldPlugin;
use hana_valence::FoldRecipe;
use hana_valence::FoldRecipeCapability as _;
use hana_valence::FoldSegmentProgress;
use hana_valence::FoldSequence;
use hana_valence::FoldSequenceBuilder;
use hana_valence::FoldSequencePlayback;
use hana_valence::FoldStage;
use hana_valence::FoldStageBegin;
use hana_valence::FoldStageEnd;
use hana_valence::FoldTarget;
use hana_valence::FoldTiming;
use hana_valence::Hinge;
use hana_valence::HingeClearance;
use hana_valence::HingePoseReported;
use hana_valence::Member;
use hana_valence::MemberBinding;
use hana_valence::Members;
use hana_valence::NoCapability;
use hana_valence::Orientation;
use hana_valence::PlannedFoldSequence;
use hana_valence::Position;
use hana_valence::Provides;
use hana_valence::QuadCell;
use hana_valence::QuadFoldGroupSelection;
use hana_valence::QuadSheet;
use hana_valence::RangeCrossing;
use hana_valence::RangeCrossings;
use hana_valence::RangeEdge;
use hana_valence::ResolvedAnchorGeometry;
use hana_valence::RetainedProviderKnowledge;
use hana_valence::SequenceCommand;
use hana_valence::SequenceCommandResponse;
use hana_valence::SequenceDirection;
use hana_valence::SequenceDriver;
use hana_valence::SequenceDriverTakeover;
use hana_valence::SequenceEasing;
use hana_valence::SequenceEasingError;
use hana_valence::SequenceEasingSample;
use hana_valence::SequenceEvaluation;
use hana_valence::SequenceMovement;
use hana_valence::SequenceOwner;
use hana_valence::SequenceOwnership;
use hana_valence::SequencePosition;
use hana_valence::SequenceScope;
use hana_valence::SequenceSourceState;
use hana_valence::SequenceStageId;
use hana_valence::SequenceTime;
use hana_valence::TriangleCell;
use hana_valence::TriangleCellOrientation;
use hana_valence::TriangleFoldGroupSelection;
use hana_valence::TriangleSheet;
use hana_valence::WindingClearance;
use hana_valence::Wrap;
use hana_valence::evaluate_fold_angle;
use hana_valence::fold_fraction;
use hana_valence::hinge_to_pose;
use hana_valence::resolve_anchors;

const MEMBER_COUNT: u8 = 6;
const ROOT_COUNT: usize = 3;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Selection;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Layout {
    Roots,
    Branches,
    Attachment,
    ProviderError,
    ForeignConnection,
    ForeignFoldGroupMember,
    RepeatedSelection,
    RepeatedCapability,
    UncoveredWinding,
}

#[derive(Clone, Debug)]
struct Provider {
    members: Vec<u8>,
    layout:  Layout,
}

impl Provider {
    fn roots(members: impl IntoIterator<Item = u8>) -> Self {
        Self {
            members: members.into_iter().collect(),
            layout:  Layout::Roots,
        }
    }

    fn branches() -> Self {
        Self {
            members: (0..MEMBER_COUNT).collect(),
            layout:  Layout::Branches,
        }
    }

    fn attachment() -> Self {
        Self {
            members: vec![0, 1],
            layout:  Layout::Attachment,
        }
    }
}

impl ArrangementProvider for Provider {
    type FoldGroupSelection = Selection;
    type Member = u8;

    fn members(&self) -> impl Iterator<Item = Self::Member> { self.members.clone().into_iter() }

    fn generate_plan(
        &self,
        members: &ArrangementMemberEntities<Self::Member>,
    ) -> Result<ArrangementPlan<Self::FoldGroupSelection>, ArrangementError> {
        match self.layout {
            Layout::Roots => ArrangementPlan::try_new(members, []),
            Layout::Branches => ArrangementPlan::try_new(
                members,
                [
                    connection(members, 1, 0),
                    connection(members, 2, 0),
                    connection(members, 4, 3),
                ],
            ),
            Layout::Attachment => {
                let member_entity = members.entity(&1)?;
                let plan = ArrangementPlan::try_new(members, [connection(members, 1, 0)])?;
                let groups = FoldGroups::from(FoldGroup::from(member_entity));
                let clearance = self.provide(&Selection, &groups, plan.connections())?;

                plan.with_fold_groups(Selection, groups)?
                    .with_capability(Selection, clearance)
            },
            Layout::ProviderError => Err(ArrangementError::provider(ProviderFailure)),
            Layout::ForeignFoldGroupMember => {
                let root_entity = members.entity(&0)?;

                ArrangementPlan::try_new(members, [connection(members, 1, 0)])?
                    .with_fold_groups(Selection, FoldGroups::from(FoldGroup::from(root_entity)))
            },
            Layout::RepeatedSelection => {
                let groups = FoldGroups::from(FoldGroup::from(members.entity(&1)?));

                ArrangementPlan::try_new(members, [connection(members, 1, 0)])?
                    .with_fold_groups(Selection, groups.clone())?
                    .with_fold_groups(Selection, groups)
            },
            Layout::RepeatedCapability => {
                let groups = FoldGroups::from(FoldGroup::from(members.entity(&1)?));
                let plan = ArrangementPlan::try_new(members, [connection(members, 1, 0)])?;
                let clearance = self.provide(&Selection, &groups, plan.connections())?;

                plan.with_fold_groups(Selection, groups)?
                    .with_capability(Selection, clearance.clone())?
                    .with_capability(Selection, clearance)
            },
            Layout::UncoveredWinding => {
                let groups = FoldGroups::from(FoldGroup::from(members.entity(&1)?));
                let plan = ArrangementPlan::try_new(members, [])?;
                let clearance = self.provide(&Selection, &groups, plan.connections())?;

                plan.with_fold_groups(Selection, groups)?
                    .with_capability(Selection, clearance)
            },
            Layout::ForeignConnection => ArrangementPlan::try_new(
                members,
                [ArrangementConnection {
                    member_entity:   Entity::PLACEHOLDER,
                    anchored_to:     AnchoredTo::new(
                        Entity::PLACEHOLDER,
                        AnchorSite::Center,
                        AnchorSite::Center,
                    ),
                    member_edge:     Edge {
                        start: AnchorSite::Vertex(0),
                        end:   AnchorSite::Vertex(1),
                    },
                    base_angle:      Angle::default(),
                    hinge_clearance: HingeClearance::CENTERED,
                }],
            ),
        }
    }
}

const WINDING_STEP: Displacement = Displacement::new(0.0, 0.25, 0.0);

impl Provides<WindingClearance> for Provider {
    fn provide(
        &self,
        _selection: &Selection,
        groups: &FoldGroups,
        connections: &[ArrangementConnection],
    ) -> Result<WindingClearance, ArrangementError> {
        WindingClearance::try_new(
            groups,
            connections
                .iter()
                .map(|connection| (connection.member_entity, WINDING_STEP)),
        )
        .map_err(ArrangementError::provider)
    }
}

/// Selection vocabulary belonging to a different provider.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ForeignSelection;

/// Capability type no provider in this test associates.
#[derive(Debug)]
struct UnprovidedCapability;

#[derive(Debug, thiserror::Error)]
#[error("provider intentionally failed")]
struct ProviderFailure;

#[derive(Component, Default, FromTemplate)]
struct SceneMarker;

#[derive(Resource)]
struct LateArrangement(Entity);

fn connection(
    members: &ArrangementMemberEntities<u8>,
    source: u8,
    target: u8,
) -> ArrangementConnection {
    let source_entity = members.entity(&source).unwrap_or(Entity::PLACEHOLDER);
    let target_entity = members.entity(&target).unwrap_or(Entity::PLACEHOLDER);
    ArrangementConnection {
        member_entity:   source_entity,
        anchored_to:     AnchoredTo::new(target_entity, AnchorSite::Center, AnchorSite::Center),
        member_edge:     Edge {
            start: AnchorSite::Vertex(0),
            end:   AnchorSite::Vertex(1),
        },
        base_angle:      Angle::default(),
        hinge_clearance: HingeClearance::CENTERED,
    }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        TaskPoolPlugin::default(),
        AssetPlugin::default(),
        ArrangementPlugin,
    ));
    app
}

fn member_entities(app: &mut App, arrangement: Entity) -> Vec<Entity> {
    app.world_mut()
        .get::<Members>(arrangement)
        .map(Members::iter)
        .map(Iterator::collect)
        .unwrap_or_default()
}

fn queue_late_arrangement(mut commands: Commands) {
    let arrangement = commands
        .spawn_arrangement(Provider::roots([0]), |_| bsn! { SceneMarker })
        .unwrap_or(Entity::PLACEHOLDER);
    commands.insert_resource(LateArrangement(arrangement));
}

fn center_geometry() -> ResolvedAnchorGeometry {
    let frame = AnchorFrame::try_new(Position::default(), Orientation::default());
    assert!(frame.is_ok());
    let geometry =
        ResolvedAnchorGeometry::try_new([(AnchorSite::Center, frame.unwrap_or_default())], []);
    assert!(geometry.is_ok());
    geometry.unwrap_or_default()
}

fn bound_attachment(app: &mut App, target: Entity, source: Entity) -> Entity {
    let mut commands = app.world_mut().commands();
    commands
        .spawn_arrangement_from_members(Provider::attachment(), |member| match *member {
            0 => MemberBinding::Bound(target),
            1 => MemberBinding::Bound(source),
            _ => MemberBinding::Missing,
        })
        .unwrap_or(Entity::PLACEHOLDER)
}

#[test]
fn code_authored_bsn_scenes_apply_to_reserved_member_roots() {
    let mut app = app();
    let arrangement = {
        let mut commands = app.world_mut().commands();
        commands
            .spawn_arrangement(Provider::roots([0, 1]), |_| bsn! { SceneMarker })
            .unwrap_or(Entity::PLACEHOLDER)
    };
    app.world_mut().flush();

    assert!(app.world().get::<Arrangement>(arrangement).is_some());
    assert_eq!(member_entities(&mut app, arrangement).len(), 2);
    assert_eq!(
        app.world_mut()
            .query::<&SceneMarker>()
            .iter(app.world())
            .count(),
        0
    );

    app.update();

    let members = member_entities(&mut app, arrangement);
    assert_eq!(members.len(), 2);
    assert!(
        members
            .iter()
            .all(|member| app.world().get::<SceneMarker>(*member).is_some())
    );
    assert_eq!(
        app.world_mut()
            .query::<&SceneMarker>()
            .iter(app.world())
            .count(),
        2
    );
}

#[test]
fn bsn_scene_queued_after_spawn_scene_applies_on_the_next_frame() {
    let mut app = app();
    app.add_systems(PostUpdate, queue_late_arrangement.run_if(run_once));

    app.update();

    let arrangement = app
        .world()
        .get_resource::<LateArrangement>()
        .map_or(Entity::PLACEHOLDER, |late| late.0);
    let members = member_entities(&mut app, arrangement);
    let member = members.first().copied().unwrap_or(Entity::PLACEHOLDER);
    assert_ne!(member, Entity::PLACEHOLDER);
    assert!(app.world().get::<SceneMarker>(member).is_none());

    app.update();

    assert!(app.world().get::<SceneMarker>(member).is_some());
    assert_eq!(
        app.world_mut()
            .query::<&SceneMarker>()
            .iter(app.world())
            .count(),
        1
    );
}

#[test]
fn empty_scenes_and_zero_member_arrangements_materialize_without_extra_roots() {
    let mut app = app();
    let initial_entities = app.world().iter_entities().count();
    let zero = {
        let mut commands = app.world_mut().commands();
        commands
            .spawn_arrangement(Provider::roots([]), |_| ())
            .unwrap_or(Entity::PLACEHOLDER)
    };
    let two = {
        let mut commands = app.world_mut().commands();
        commands
            .spawn_arrangement(Provider::roots([4, 9]), |_| ())
            .unwrap_or(Entity::PLACEHOLDER)
    };
    app.world_mut().flush();

    assert!(app.world().get::<Arrangement>(zero).is_some());
    assert!(
        app.world()
            .get::<Members>(zero)
            .is_some_and(Members::is_empty)
    );
    assert_eq!(member_entities(&mut app, two).len(), 2);
    assert_eq!(app.world().iter_entities().count(), initial_entities + 4);
}

#[test]
fn materialization_preserves_membership_order_and_independent_physical_forest() {
    let mut app = app();
    let arrangement = {
        let mut commands = app.world_mut().commands();
        commands
            .spawn_arrangement(Provider::branches(), |_| ())
            .unwrap_or(Entity::PLACEHOLDER)
    };
    app.world_mut().flush();

    let members = member_entities(&mut app, arrangement);
    assert_eq!(members.len(), usize::from(MEMBER_COUNT));
    assert_eq!(
        members
            .iter()
            .filter(|member| app.world().get::<AnchoredTo>(**member).is_none())
            .count(),
        ROOT_COUNT
    );
    assert_eq!(
        app.world()
            .get::<AnchoredTo>(members[1])
            .map(AnchoredTo::target),
        Some(members[0]),
    );
    assert_eq!(
        app.world()
            .get::<AnchoredTo>(members[2])
            .map(AnchoredTo::target),
        Some(members[0]),
    );
    assert_eq!(
        app.world()
            .get::<AnchoredTo>(members[4])
            .map(AnchoredTo::target),
        Some(members[3]),
    );
    assert_eq!(
        app.world()
            .get::<Member>(members[5])
            .map(|member| member.arrangement_entity),
        Some(arrangement)
    );
}

#[test]
fn existing_member_bindings_validate_before_materialization() {
    let mut app = app();
    let bound = [
        app.world_mut().spawn_empty().id(),
        app.world_mut().spawn_empty().id(),
        app.world_mut().spawn_empty().id(),
    ];
    let arrangement = {
        let mut commands = app.world_mut().commands();
        commands
            .spawn_arrangement_from_members(Provider::roots([0, 1, 2]), |member| {
                MemberBinding::Bound(bound[usize::from(*member)])
            })
            .unwrap_or(Entity::PLACEHOLDER)
    };
    app.world_mut().flush();

    assert_eq!(member_entities(&mut app, arrangement), bound);
    assert!(bound.iter().all(|entity| {
        app.world()
            .get::<Member>(*entity)
            .is_some_and(|member| member.arrangement_entity == arrangement)
    }));
}

#[test]
fn disappearing_bound_connection_source_is_not_recreated_during_materialization() {
    let mut app = app();
    let target = app.world_mut().spawn_empty().id();
    let source = app.world_mut().spawn_empty().id();
    let arrangement = bound_attachment(&mut app, target, source);

    app.world_mut().entity_mut(source).despawn();
    app.world_mut().flush();

    assert!(app.world().get_entity(source).is_err());
    assert_eq!(member_entities(&mut app, arrangement), vec![target]);
    assert_eq!(
        app.world()
            .get::<Member>(target)
            .map(|member| member.arrangement_entity),
        Some(arrangement)
    );
}

#[test]
fn disappearing_bound_physical_target_leaves_surviving_members_best_effort() {
    let mut app = app();
    let target = app.world_mut().spawn_empty().id();
    let source = app.world_mut().spawn_empty().id();
    let arrangement = bound_attachment(&mut app, target, source);

    app.world_mut().entity_mut(target).despawn();
    app.world_mut().flush();

    assert!(app.world().get_entity(target).is_err());
    assert_eq!(member_entities(&mut app, arrangement), vec![source]);
    assert_eq!(
        app.world()
            .get::<Member>(source)
            .map(|member| member.arrangement_entity),
        Some(arrangement)
    );
}

#[test]
fn missing_and_duplicate_bindings_consume_every_member_then_clean_up_controller() {
    let mut app = app();
    let bound = app.world_mut().spawn_empty().id();
    let initial_entities = app.world().iter_entities().count();
    let calls = Cell::new(0);
    let missing = {
        let mut commands = app.world_mut().commands();
        commands.spawn_arrangement_from_members(Provider::roots([0, 1, 2]), |_| {
            calls.set(calls.get() + 1);
            MemberBinding::Missing
        })
    };
    assert!(matches!(
        missing,
        Err(ArrangementError::MissingMemberBinding { .. })
    ));
    assert_eq!(calls.get(), 3);
    app.world_mut().flush();
    assert_eq!(app.world().iter_entities().count(), initial_entities);

    let duplicate = {
        let mut commands = app.world_mut().commands();
        commands.spawn_arrangement_from_members(Provider::roots([0, 1]), |_| {
            MemberBinding::Bound(bound)
        })
    };
    assert!(
        matches!(duplicate, Err(ArrangementError::DuplicateMemberEntity { member_entity }) if member_entity == bound)
    );
    app.world_mut().flush();
    assert_eq!(app.world().iter_entities().count(), initial_entities);
}

#[test]
fn synchronous_failures_preserve_errors_skip_scenes_and_clean_reserved_entities() {
    let mut app = app();
    let initial_entities = app.world().iter_entities().count();
    let scenes = Cell::new(0);
    let duplicate = {
        let mut commands = app.world_mut().commands();
        commands.spawn_arrangement(Provider::roots([0, 0]), |_| {
            scenes.set(scenes.get() + 1);
        })
    };
    assert!(matches!(
        duplicate,
        Err(ArrangementError::DuplicateLogicalMember { member }) if member == "0"
    ));

    let provider_error = {
        let mut commands = app.world_mut().commands();
        commands.spawn_arrangement(
            Provider {
                members: vec![0],
                layout:  Layout::ProviderError,
            },
            |_| {
                scenes.set(scenes.get() + 1);
            },
        )
    };
    assert!(matches!(
        &provider_error,
        Err(ArrangementError::Provider { .. })
    ));
    assert_eq!(
        provider_error
            .as_ref()
            .err()
            .and_then(|error| core::error::Error::source(error))
            .map(ToString::to_string),
        Some(String::from("provider intentionally failed")),
    );

    let plan_error = {
        let mut commands = app.world_mut().commands();
        commands.spawn_arrangement(
            Provider {
                members: vec![0],
                layout:  Layout::ForeignConnection,
            },
            |_| {
                scenes.set(scenes.get() + 1);
            },
        )
    };
    assert!(matches!(
        plan_error,
        Err(ArrangementError::UnlistedConnectionSource { member_entity })
            if member_entity == Entity::PLACEHOLDER
    ));
    assert_eq!(scenes.get(), 0);
    app.world_mut().flush();
    assert_eq!(app.world().iter_entities().count(), initial_entities);
}

fn failing_spawn(
    app: &mut App,
    layout: Layout,
    scenes: &Cell<usize>,
) -> Result<Entity, ArrangementError> {
    let mut commands = app.world_mut().commands();
    commands.spawn_arrangement(
        Provider {
            members: vec![0, 1],
            layout,
        },
        |_| {
            scenes.set(scenes.get() + 1);
        },
    )
}

#[test]
fn fold_group_and_capability_failures_clean_reserved_entities_before_materialization() {
    let mut app = app();
    let initial_entities = app.world().iter_entities().count();
    let scenes = Cell::new(0);

    assert!(matches!(
        failing_spawn(&mut app, Layout::ForeignFoldGroupMember, &scenes),
        Err(ArrangementError::ForeignFoldGroupMember { .. })
    ));
    assert!(matches!(
        failing_spawn(&mut app, Layout::RepeatedSelection, &scenes),
        Err(ArrangementError::DuplicateFoldGroupSelection { selection })
            if selection == "Selection"
    ));
    assert!(matches!(
        failing_spawn(&mut app, Layout::RepeatedCapability, &scenes),
        Err(ArrangementError::DuplicateCapability { selection, capability })
            if selection == "Selection" && capability.ends_with("WindingClearance")
    ));

    let uncovered_winding = failing_spawn(&mut app, Layout::UncoveredWinding, &scenes);
    assert!(matches!(
        &uncovered_winding,
        Err(ArrangementError::Provider { .. })
    ));
    assert!(matches!(
        uncovered_winding
            .as_ref()
            .err()
            .and_then(|error| core::error::Error::source(error))
            .and_then(|source| source.downcast_ref::<FoldAuthorError>()),
        Some(FoldAuthorError::MissingWindingClearance { .. })
    ));

    assert_eq!(scenes.get(), 0);
    app.world_mut().flush();
    assert_eq!(app.world().iter_entities().count(), initial_entities);
}

#[test]
fn member_and_attachment_retargeting_update_independent_reverse_relationships() {
    let mut app = app();
    let (first, second) = {
        let mut commands = app.world_mut().commands();
        let first = commands
            .spawn_arrangement(Provider::roots([0]), |_| ())
            .unwrap_or(Entity::PLACEHOLDER);
        let second = commands
            .spawn_arrangement(Provider::roots([]), |_| ())
            .unwrap_or(Entity::PLACEHOLDER);
        (first, second)
    };
    app.world_mut().flush();

    let target_a = app.world_mut().spawn_empty().id();
    let target_b = app.world_mut().spawn_empty().id();
    let member = member_entities(&mut app, first)
        .into_iter()
        .next()
        .unwrap_or(Entity::PLACEHOLDER);
    assert_ne!(member, Entity::PLACEHOLDER);

    app.world_mut()
        .entity_mut(member)
        .insert(Member::new(second));
    app.world_mut().entity_mut(member).insert(AnchoredTo::new(
        target_a,
        AnchorSite::Center,
        AnchorSite::Center,
    ));
    app.world_mut().entity_mut(member).insert(AnchoredTo::new(
        target_b,
        AnchorSite::Center,
        AnchorSite::Center,
    ));

    assert!(
        app.world()
            .get::<Members>(first)
            .is_none_or(Members::is_empty)
    );
    assert_eq!(member_entities(&mut app, second), vec![member]);
    assert!(
        app.world()
            .get::<AnchoredHere>(target_a)
            .is_none_or(AnchoredHere::is_empty)
    );
    assert_eq!(
        app.world()
            .get::<AnchoredHere>(target_b)
            .map(AnchoredHere::iter)
            .map(Iterator::collect::<Vec<_>>),
        Some(vec![member]),
    );
}

#[test]
fn arrangement_and_fold_plugins_leave_one_application_owned_resolver_for_late_inputs() {
    let mut app = App::new();
    app.add_plugins((
        TaskPoolPlugin::default(),
        AssetPlugin::default(),
        TransformPlugin,
        ArrangementPlugin,
        FoldPlugin,
    ))
    .insert_resource(Time::<Virtual>::default())
    .insert_resource(AnchorResolveDiagnostics::default())
    .configure_sets(
        PostUpdate,
        (
            AnchorSystems::FillGeometry,
            AnchorSystems::AnimatePose,
            AnchorSystems::Resolve,
        )
            .chain()
            .before(TransformSystems::Propagate),
    )
    .add_systems(PostUpdate, resolve_anchors.in_set(AnchorSystems::Resolve));

    assert!(app.is_plugin_added::<ScenePlugin>());

    let target_transform = Transform::from_translation(Vec3::new(3.0, 0.0, 0.0));
    let target = app
        .world_mut()
        .spawn((target_transform, GlobalTransform::from(target_transform)))
        .id();
    let source_transform = Transform::from_translation(Vec3::new(-2.0, 0.0, 0.0));
    let source = app
        .world_mut()
        .spawn((
            center_geometry(),
            source_transform,
            GlobalTransform::from(source_transform),
        ))
        .id();
    let arrangement = bound_attachment(&mut app, target, source);
    app.world_mut().flush();

    app.update();

    let resolver_count = app
        .get_schedule(PostUpdate)
        .and_then(|schedule| schedule.systems().ok())
        .map(|systems| {
            systems
                .filter(|(_, system)| system.name().contains("resolve_anchors"))
                .count()
        })
        .unwrap_or_default();
    assert_eq!(resolver_count, 1);
    assert_eq!(
        app.world()
            .get::<AnchoredTo>(source)
            .map(AnchoredTo::target),
        Some(target)
    );
    assert_eq!(
        app.world()
            .get::<Transform>(source)
            .map(|transform| transform.translation),
        Some(source_transform.translation)
    );

    app.world_mut().entity_mut(target).insert(center_geometry());
    app.update();

    assert_eq!(
        app.world()
            .get::<Transform>(source)
            .map(|transform| transform.translation),
        Some(target_transform.translation)
    );
    assert!(app.world().get::<Member>(source).is_some());
    assert!(app.world().get::<Members>(arrangement).is_some());
}

fn folded_member(app: &App, arrangement: Entity) -> Entity {
    let Ok(retained) = RetainedProviderKnowledge::for_arrangement(app.world(), arrangement) else {
        return Entity::PLACEHOLDER;
    };
    let Ok(groups) = retained.groups(&Selection) else {
        return Entity::PLACEHOLDER;
    };
    assert_eq!(groups.as_slice().len(), 1);

    groups[0]
        .iter()
        .copied()
        .next()
        .unwrap_or(Entity::PLACEHOLDER)
}

fn winding_step(app: &App, arrangement: Entity, member: Entity) -> Displacement {
    let Ok(retained) = RetainedProviderKnowledge::for_arrangement(app.world(), arrangement) else {
        return Displacement::default();
    };
    let Ok(clearance) = retained.capability::<Selection, WindingClearance>(&Selection) else {
        return Displacement::default();
    };

    clearance
        .clearance_for(member)
        .unwrap_or_else(|_| Displacement::default())
}

#[test]
fn both_construction_commands_retain_the_same_groups_and_capabilities() {
    let mut app = app();
    let target = app.world_mut().spawn_empty().id();
    let source = app.world_mut().spawn_empty().id();
    let bound = bound_attachment(&mut app, target, source);
    let spawned = {
        let mut commands = app.world_mut().commands();
        commands
            .spawn_arrangement(Provider::attachment(), |_| ())
            .unwrap_or(Entity::PLACEHOLDER)
    };
    app.world_mut().flush();

    let spawned_member = folded_member(&app, spawned);

    assert_eq!(folded_member(&app, bound), source);
    assert!(app.world().get::<AnchoredTo>(spawned_member).is_some());
    assert_eq!(winding_step(&app, bound, source), WINDING_STEP);
    assert_eq!(winding_step(&app, spawned, spawned_member), WINDING_STEP);
}

#[test]
fn a_retained_read_names_a_foreign_selection_type_and_a_missing_capability() {
    let mut app = app();
    let target = app.world_mut().spawn_empty().id();
    let source = app.world_mut().spawn_empty().id();
    let arrangement = bound_attachment(&mut app, target, source);
    app.world_mut().flush();
    let retained = valid(
        "retained provider knowledge",
        RetainedProviderKnowledge::for_arrangement(app.world(), arrangement),
    );

    assert!(matches!(
        retained.groups(&ForeignSelection),
        Err(ArrangementError::MismatchedFoldGroupSelectionType { expected, found })
            if expected.ends_with("Selection") && found.ends_with("ForeignSelection"),
    ));
    assert!(matches!(
        retained.capability::<Selection, UnprovidedCapability>(&Selection),
        Err(ArrangementError::MissingCapability { capability, .. })
            if capability.ends_with("UnprovidedCapability"),
    ));
}

#[test]
fn a_provider_without_fold_groups_retains_no_selection_value() {
    let mut app = app();
    let arrangement = {
        let mut commands = app.world_mut().commands();
        commands
            .spawn_arrangement(Provider::roots([0]), |_| ())
            .unwrap_or(Entity::PLACEHOLDER)
    };
    app.world_mut().flush();
    let retained = valid(
        "retained provider knowledge",
        RetainedProviderKnowledge::for_arrangement(app.world(), arrangement),
    );

    assert!(matches!(
        retained.groups(&Selection),
        Err(ArrangementError::UnknownFoldGroupSelection { .. }),
    ));
}

// Built-in sheet providers.

const HEX_PETALS: u8 = 6;

/// Example-local hex flower proving the trait extends without a new core type.
struct HexFlower;

/// The one hex alternative: every petal folds toward the center tile.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct HexPetalSelection;

impl ArrangementProvider for HexFlower {
    type FoldGroupSelection = HexPetalSelection;
    type Member = u8;

    fn members(&self) -> impl Iterator<Item = Self::Member> { 0..=HEX_PETALS }

    fn generate_plan(
        &self,
        members: &ArrangementMemberEntities<Self::Member>,
    ) -> Result<ArrangementPlan<Self::FoldGroupSelection>, ArrangementError> {
        let mut connections = Vec::new();
        let mut petals = Vec::new();
        for petal in 1..=HEX_PETALS {
            connections.push(connection(members, petal, 0));
            petals.push(members.entity(&petal)?);
        }
        let group = FoldGroup::try_from_iter(petals).map_err(ArrangementError::provider)?;

        ArrangementPlan::try_new(members, connections)?
            .with_fold_groups(HexPetalSelection, FoldGroups::from(group))
    }
}

/// Example-local box net proving a staged downstream forest needs no core type.
struct BoxNet;

/// The one box alternative: the four sides, then the far face.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct BoxSideSelection;

impl ArrangementProvider for BoxNet {
    type FoldGroupSelection = BoxSideSelection;
    type Member = u8;

    fn members(&self) -> impl Iterator<Item = Self::Member> { 0..6 }

    fn generate_plan(
        &self,
        members: &ArrangementMemberEntities<Self::Member>,
    ) -> Result<ArrangementPlan<Self::FoldGroupSelection>, ArrangementError> {
        let mut connections = Vec::new();
        let mut sides = Vec::new();
        for side in 1..5 {
            connections.push(connection(members, side, 0));
            sides.push(members.entity(&side)?);
        }
        connections.push(connection(members, 5, 4));
        let sides = FoldGroup::try_from_iter(sides).map_err(ArrangementError::provider)?;
        let far_face = FoldGroup::from(members.entity(&5)?);

        ArrangementPlan::try_new(members, connections)?
            .with_fold_groups(BoxSideSelection, FoldGroups::new(sides, [far_face]))
    }
}

/// Example-local provider with no fold alternative at all.
struct LooseTiles;

impl ArrangementProvider for LooseTiles {
    type FoldGroupSelection = Infallible;
    type Member = u8;

    fn members(&self) -> impl Iterator<Item = Self::Member> { 0..3 }

    fn generate_plan(
        &self,
        members: &ArrangementMemberEntities<Self::Member>,
    ) -> Result<ArrangementPlan<Self::FoldGroupSelection>, ArrangementError> {
        ArrangementPlan::try_new(members, [])
    }
}

fn try_spawn_sheet(
    app: &mut App,
    provider: impl ArrangementProvider,
) -> Result<Entity, ArrangementError> {
    let arrangement = {
        let mut commands = app.world_mut().commands();
        commands.spawn_arrangement(provider, |_| ())
    };
    app.world_mut().flush();

    arrangement
}

fn spawn_sheet(app: &mut App, provider: impl ArrangementProvider) -> Entity {
    let arrangement = try_spawn_sheet(app, provider);
    assert!(arrangement.is_ok(), "provider rejected its own plan");

    arrangement.unwrap_or(Entity::PLACEHOLDER)
}

fn has_alternative<S>(app: &App, arrangement: Entity, selection: &S) -> bool
where
    S: Eq + Hash + Debug + Send + Sync + 'static,
{
    RetainedProviderKnowledge::for_arrangement(app.world(), arrangement)
        .is_ok_and(|retained| retained.groups(selection).is_ok())
}

fn root_members(app: &App, members: &[Entity]) -> Vec<Entity> {
    members
        .iter()
        .copied()
        .filter(|member| app.world().get::<AnchoredTo>(*member).is_none())
        .collect()
}

fn retained_groups<S>(app: &App, arrangement: Entity, selection: &S) -> Vec<Vec<Entity>>
where
    S: Eq + Hash + Debug + Send + Sync + 'static,
{
    let Ok(retained) = RetainedProviderKnowledge::for_arrangement(app.world(), arrangement) else {
        return Vec::new();
    };
    let Ok(groups) = retained.groups(selection) else {
        return Vec::new();
    };

    groups
        .iter()
        .map(|group| group.iter().copied().collect())
        .collect()
}

/// Asserts one alternative covers every non-root member exactly once.
fn assert_complete_alternative<S>(
    app: &App,
    arrangement: Entity,
    selection: &S,
    members: &[Entity],
) -> Vec<Vec<Entity>>
where
    S: Eq + Hash + Debug + Send + Sync + 'static,
{
    let groups = retained_groups(app, arrangement, selection);
    let roots = root_members(app, members);
    let covered = groups.concat();
    let mut unique = covered.clone();
    unique.sort_unstable();
    unique.dedup();

    assert_eq!(
        unique.len(),
        covered.len(),
        "{selection:?} repeats a member"
    );
    assert_eq!(
        covered.len(),
        members.len() - roots.len(),
        "{selection:?} misses a hinged member",
    );
    assert!(
        roots.iter().all(|root| !covered.contains(root)),
        "{selection:?} folds a forest root",
    );

    groups
}

/// Asserts every member of every group carries exactly `layer` of clearance.
fn assert_uniform_winding<S>(
    app: &App,
    arrangement: Entity,
    selection: &S,
    groups: &[Vec<Entity>],
    layer: Displacement,
) where
    S: Eq + Hash + Debug + Send + Sync + 'static,
{
    let retained = RetainedProviderKnowledge::for_arrangement(app.world(), arrangement);
    assert!(
        retained.is_ok(),
        "arrangement retained no provider knowledge"
    );
    let Ok(retained) = retained else { return };

    let clearance = retained.capability::<S, WindingClearance>(selection);
    assert!(
        clearance.is_ok(),
        "{selection:?} retained no winding clearance"
    );
    let Ok(clearance) = clearance else { return };

    assert!(
        groups.iter().flatten().all(|member| clearance
            .clearance_for(*member)
            .is_ok_and(|clearance| clearance == layer)),
        "{selection:?} did not give every member one layer of thickness",
    );
}

/// Returns the source and target sites the materialized crease names.
fn crease_anchors(app: &App, member: Entity) -> Option<(AnchorSite, AnchorSite)> {
    app.world()
        .get::<AnchoredTo>(member)
        .map(|anchored| (anchored.source_anchor(), anchored.target_anchor()))
}

#[test]
fn quad_sheet_alternatives_cover_every_non_root_cell_and_carry_winding() {
    let mut app = app();
    let quad_sheet = QuadSheet::new(3, 4);
    let arrangement = spawn_sheet(&mut app, quad_sheet);
    let members = member_entities(&mut app, arrangement);

    assert_eq!(members.len(), quad_sheet.rows() * quad_sheet.columns());
    assert_eq!(root_members(&app, &members), vec![members[0]]);

    let rows =
        assert_complete_alternative(&app, arrangement, &QuadFoldGroupSelection::Rows, &members);
    let columns = assert_complete_alternative(
        &app,
        arrangement,
        &QuadFoldGroupSelection::Columns,
        &members,
    );

    assert_eq!(rows.len(), quad_sheet.rows());
    assert_eq!(columns.len(), quad_sheet.columns());
    assert_eq!(rows[0].len(), quad_sheet.columns() - 1);
    assert_eq!(rows[1].len(), quad_sheet.columns());
    assert_eq!(columns[0].len(), quad_sheet.rows() - 1);
    assert_eq!(columns[1].len(), quad_sheet.rows());
    let layer = Displacement::new(0.0, 0.0, QuadSheet::LAYER_THICKNESS);
    assert_uniform_winding(
        &app,
        arrangement,
        &QuadFoldGroupSelection::Rows,
        &rows,
        layer,
    );
    assert_uniform_winding(
        &app,
        arrangement,
        &QuadFoldGroupSelection::Columns,
        &columns,
        layer,
    );
}

#[test]
fn triangle_sheet_alternatives_cover_every_non_root_cell_and_carry_winding() {
    let mut app = app();
    let triangle_sheet = TriangleSheet::new(3, 4);
    let arrangement = spawn_sheet(&mut app, triangle_sheet);
    let members = member_entities(&mut app, arrangement);

    assert_eq!(
        members.len(),
        triangle_sheet.rows() * triangle_sheet.columns()
    );
    assert_eq!(root_members(&app, &members), vec![members[0]]);

    let rows = assert_complete_alternative(
        &app,
        arrangement,
        &TriangleFoldGroupSelection::Rows,
        &members,
    );
    let columns = assert_complete_alternative(
        &app,
        arrangement,
        &TriangleFoldGroupSelection::Columns,
        &members,
    );

    assert_eq!(rows.len(), triangle_sheet.rows());
    assert_eq!(columns.len(), triangle_sheet.columns());
    let layer = Displacement::new(0.0, 0.0, TriangleSheet::LAYER_THICKNESS);
    assert_uniform_winding(
        &app,
        arrangement,
        &TriangleFoldGroupSelection::Rows,
        &rows,
        layer,
    );
    assert_uniform_winding(
        &app,
        arrangement,
        &TriangleFoldGroupSelection::Columns,
        &columns,
        layer,
    );
}

#[test]
fn every_triangle_crease_names_the_edge_index_both_cells_share() {
    let mut app = app();
    let pair = spawn_sheet(&mut app, TriangleSheet::new(1, 2));
    let pair_members = member_entities(&mut app, pair);

    // Cell (0, 1) points downward, so the edge it shares with the upward cell
    // (0, 0) on its left is index zero on both cells.
    assert_eq!(
        app.world()
            .get::<AnchoredTo>(pair_members[1])
            .map(AnchoredTo::target),
        Some(pair_members[0]),
    );
    assert_eq!(
        crease_anchors(&app, pair_members[1]),
        Some((AnchorSite::EdgeMidpoint(0), AnchorSite::EdgeMidpoint(0))),
    );

    let sheet = spawn_sheet(&mut app, TriangleSheet::new(3, 2));
    let members = member_entities(&mut app, sheet);

    // Row one links upward at column zero: (1, 0) attaches through its
    // horizontal base edge, and (1, 1) chains leftward from it.
    assert_eq!(
        crease_anchors(&app, members[2]),
        Some((AnchorSite::EdgeMidpoint(1), AnchorSite::EdgeMidpoint(1))),
    );
    assert_eq!(
        crease_anchors(&app, members[3]),
        Some((AnchorSite::EdgeMidpoint(2), AnchorSite::EdgeMidpoint(2))),
    );

    // Row two links upward at column one, so (2, 0) creases rightward.
    assert_eq!(
        app.world()
            .get::<AnchoredTo>(members[4])
            .map(AnchoredTo::target),
        Some(members[5]),
    );
    assert_eq!(
        crease_anchors(&app, members[4]),
        Some((AnchorSite::EdgeMidpoint(0), AnchorSite::EdgeMidpoint(0))),
    );
}

#[test]
fn every_quad_crease_names_the_facing_midpoint_pair() {
    let mut app = app();
    let arrangement = spawn_sheet(&mut app, QuadSheet::new(2, 2));
    let members = member_entities(&mut app, arrangement);

    // Cell (0, 1) seats its left midpoint against the right midpoint of (0, 0).
    assert_eq!(
        crease_anchors(&app, members[1]),
        Some((AnchorSite::EdgeMidpoint(3), AnchorSite::EdgeMidpoint(1))),
    );

    // Cell (1, 0) seats its top midpoint against the bottom midpoint of (0, 0).
    assert_eq!(
        crease_anchors(&app, members[2]),
        Some((AnchorSite::EdgeMidpoint(0), AnchorSite::EdgeMidpoint(2))),
    );
    assert_eq!(
        app.world()
            .get::<AnchoredTo>(members[2])
            .map(AnchoredTo::target),
        Some(members[0]),
    );
}

#[test]
fn one_row_and_one_column_sheets_yield_strip_groups_of_one_less_than_their_cells() {
    let mut app = app();
    let cells = 5;
    for (rows, columns) in [(1, cells), (cells, 1)] {
        let arrangement = spawn_sheet(&mut app, QuadSheet::new(rows, columns));
        let members = member_entities(&mut app, arrangement);
        assert_eq!(members.len(), cells);

        let row_groups =
            assert_complete_alternative(&app, arrangement, &QuadFoldGroupSelection::Rows, &members);
        let column_groups = assert_complete_alternative(
            &app,
            arrangement,
            &QuadFoldGroupSelection::Columns,
            &members,
        );

        assert_eq!(row_groups.concat().len(), cells - 1);
        assert_eq!(column_groups.concat().len(), cells - 1);
        let (single, split) = if rows == 1 {
            (row_groups, column_groups)
        } else {
            (column_groups, row_groups)
        };
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].len(), cells - 1);
        assert_eq!(split.len(), cells - 1);
        assert!(split.iter().all(|group| group.len() == 1));
    }
}

#[test]
fn a_single_column_triangle_sheet_leaves_its_unlinked_rows_as_forest_roots() {
    let mut app = app();
    let triangle_sheet = TriangleSheet::new(4, 1);
    let arrangement = spawn_sheet(&mut app, triangle_sheet);
    let members = member_entities(&mut app, arrangement);

    assert_eq!(root_members(&app, &members), vec![members[0], members[2]]);

    let rows = assert_complete_alternative(
        &app,
        arrangement,
        &TriangleFoldGroupSelection::Rows,
        &members,
    );
    let columns = assert_complete_alternative(
        &app,
        arrangement,
        &TriangleFoldGroupSelection::Columns,
        &members,
    );

    assert_eq!(rows.len(), 2);
    assert_eq!(columns.len(), 1);
    assert_eq!(columns[0], vec![members[1], members[3]]);
}

#[test]
fn a_single_cell_sheet_has_no_crease_and_retains_no_alternative() {
    let mut app = app();
    for (rows, columns) in [(1, 1), (0, 0), (0, 3)] {
        let quad = spawn_sheet(&mut app, QuadSheet::new(rows, columns));
        let triangle = spawn_sheet(&mut app, TriangleSheet::new(rows, columns));

        assert!(!has_alternative(&app, quad, &QuadFoldGroupSelection::Rows));
        assert!(!has_alternative(
            &app,
            quad,
            &QuadFoldGroupSelection::Columns
        ));
        assert!(!has_alternative(
            &app,
            triangle,
            &TriangleFoldGroupSelection::Rows,
        ));
        assert!(!has_alternative(
            &app,
            triangle,
            &TriangleFoldGroupSelection::Columns,
        ));
    }
}

#[test]
fn built_in_sheets_build_every_dimension_without_a_foreign_fold_group_member() {
    let mut app = app();
    for (rows, columns) in [(1, 1), (1, 5), (5, 1), (2, 3), (3, 2), (4, 4)] {
        let quad = try_spawn_sheet(&mut app, QuadSheet::new(rows, columns));
        let triangle = try_spawn_sheet(&mut app, TriangleSheet::new(rows, columns));

        assert!(quad.is_ok(), "quad {rows}x{columns} failed: {quad:?}");
        assert!(
            triangle.is_ok(),
            "triangle {rows}x{columns} failed: {triangle:?}",
        );
    }
}

fn fold_timing() -> FoldTiming { FoldTiming::new(Duration::from_secs(1), EaseFunction::Linear) }

fn authored_sequence_count(app: &mut App) -> usize {
    let mut authored = app.world_mut().query::<&FoldSequence>();
    authored.iter(app.world()).count()
}

fn arrangement_count(app: &mut App) -> usize {
    let mut arrangements = app.world_mut().query::<&Arrangement>();
    arrangements.iter(app.world()).count()
}

#[test]
fn a_creaseless_sheet_names_its_unretained_selection_and_authors_no_sequence() {
    let mut app = app();
    for (rows, columns) in [(1, 1), (0, 3), (3, 0), (0, 0)] {
        let quad = try_spawn_sheet(
            &mut app,
            QuadSheet::new(rows, columns)
                .with_fold_sequence(QuadFoldGroupSelection::Rows, fold_timing()),
        );
        let triangle = try_spawn_sheet(
            &mut app,
            TriangleSheet::new(rows, columns)
                .with_fold_sequence(TriangleFoldGroupSelection::Columns, fold_timing()),
        );

        for outcome in [quad, triangle] {
            let Err(ArrangementError::UnknownFoldGroupSelection { .. }) = outcome else {
                panic!("{rows}x{columns} authored a sequence with no crease: {outcome:?}");
            };
        }
    }

    assert_eq!(authored_sequence_count(&mut app), 0);
}

#[test]
fn a_provider_authored_sequence_materializes_only_on_the_controller_that_authored_it() {
    let mut app = app();
    let plain = spawn_sheet(&mut app, QuadSheet::new(2, 2));
    let staged = spawn_sheet(
        &mut app,
        QuadSheet::new(2, 2).with_fold_sequence(QuadFoldGroupSelection::Rows, fold_timing()),
    );
    let combined = spawn_sheet(
        &mut app,
        QuadSheet::new(2, 2).with_custom_fold_sequence(
            QuadFoldGroupSelection::Rows,
            |groups: &FoldGroups| {
                Ok(FoldSequenceBuilder::new(fold_timing())
                    .stage(FoldGroup::combine(groups.iter())?)
                    .build())
            },
        ),
    );

    assert!(app.world().get::<FoldSequence>(plain).is_none());
    let Some(staged_sequence) = app.world().get::<FoldSequence>(staged) else {
        panic!("the standard adapter authored no sequence on its controller");
    };
    let Some(combined_sequence) = app.world().get::<FoldSequence>(combined) else {
        panic!("the custom adapter authored no sequence on its controller");
    };

    assert_eq!(staged_sequence.stages().len(), 2);
    assert_eq!(staged_sequence.tracks().len(), 3);
    assert_eq!(staged_sequence.default_timing(), &fold_timing());
    assert_eq!(combined_sequence.stages().len(), 1);
    assert_eq!(combined_sequence.tracks().len(), 3);
    assert_eq!(authored_sequence_count(&mut app), 2);
}

#[test]
fn a_failing_sequence_closure_aborts_the_spawn_and_writes_nothing() {
    let mut app = app();
    let arrangements_before = arrangement_count(&mut app);
    let outcome = try_spawn_sheet(
        &mut app,
        QuadSheet::new(2, 2).with_custom_fold_sequence(QuadFoldGroupSelection::Rows, |_| {
            Err(FoldAuthorError::EmptyFoldGroups)
        }),
    );

    let Err(ArrangementError::Provider { source }) = &outcome else {
        panic!("a failing closure must abort the spawn: {outcome:?}");
    };
    assert_eq!(
        source.downcast_ref::<FoldAuthorError>(),
        Some(&FoldAuthorError::EmptyFoldGroups),
    );
    assert_eq!(arrangement_count(&mut app), arrangements_before);
    assert_eq!(authored_sequence_count(&mut app), 0);
}

/// Counts how many of its inner provider's plans name an authored sequence.
struct RecordPlannedSequence<P> {
    provider: P,
    authored: Arc<AtomicUsize>,
}

impl<P> ArrangementProvider for RecordPlannedSequence<P>
where
    P: ArrangementProvider,
{
    type FoldGroupSelection = P::FoldGroupSelection;
    type Member = P::Member;

    fn members(&self) -> impl Iterator<Item = Self::Member> { self.provider.members() }

    fn generate_plan(
        &self,
        members: &ArrangementMemberEntities<Self::Member>,
    ) -> Result<ArrangementPlan<Self::FoldGroupSelection>, ArrangementError> {
        let plan = self.provider.generate_plan(members)?;
        if matches!(plan.fold_sequence(), PlannedFoldSequence::Authored(_)) {
            self.authored.fetch_add(1, Ordering::Relaxed);
        }

        Ok(plan)
    }
}

#[test]
fn a_generated_plan_names_the_authored_and_unauthored_states_it_carries() {
    let mut app = app();
    let authored = Arc::new(AtomicUsize::new(0));

    spawn_sheet(
        &mut app,
        RecordPlannedSequence {
            provider: QuadSheet::new(2, 2),
            authored: Arc::clone(&authored),
        },
    );
    assert_eq!(authored.load(Ordering::Relaxed), 0);

    spawn_sheet(
        &mut app,
        RecordPlannedSequence {
            provider: QuadSheet::new(2, 2)
                .with_fold_sequence(QuadFoldGroupSelection::Rows, fold_timing()),
            authored: Arc::clone(&authored),
        },
    );
    assert_eq!(authored.load(Ordering::Relaxed), 1);
}

#[test]
fn sheet_cell_geometry_is_constructor_validated_with_provider_ordered_edges() {
    let quad_sheet = QuadSheet::new(2, 2);
    let quad_geometry = quad_sheet.cell_geometry();
    let repeated_quad_geometry = quad_sheet.cell_geometry();
    assert!(quad_geometry.is_ok(), "quad cell geometry was rejected");
    let (Ok(quad_geometry), Ok(repeated_quad_geometry)) = (quad_geometry, repeated_quad_geometry)
    else {
        return;
    };

    assert_eq!(quad_geometry.frames().count(), 9);
    assert_eq!(quad_geometry.edges(), repeated_quad_geometry.edges());
    assert_eq!(
        quad_geometry.edges().first().map(|edge| edge.start),
        Some(AnchorSite::Vertex(0)),
    );
    assert!(quad_geometry.frame(AnchorSite::Center).is_ok());
    assert!(quad_geometry.frame(AnchorSite::EdgeMidpoint(3)).is_ok());

    let triangle_sheet = TriangleSheet::new(1, 2);
    let upward = TriangleCell::new(0, 0);
    let downward = TriangleCell::new(0, 1);
    assert_eq!(upward.orientation(), TriangleCellOrientation::Upward);
    assert_eq!(downward.orientation(), TriangleCellOrientation::Downward);

    let upward_geometry = triangle_sheet.cell_geometry(upward);
    let downward_geometry = triangle_sheet.cell_geometry(downward);
    assert!(
        upward_geometry.is_ok(),
        "triangle cell geometry was rejected"
    );
    let (Ok(upward_geometry), Ok(downward_geometry)) = (upward_geometry, downward_geometry) else {
        return;
    };

    assert_eq!(upward_geometry.frames().count(), 7);
    assert_eq!(upward_geometry.edges(), downward_geometry.edges());
    assert_eq!(upward_geometry.edges().len(), 3);
    assert_ne!(
        upward_geometry
            .frame(AnchorSite::Vertex(0))
            .ok()
            .map(AnchorFrame::position),
        downward_geometry
            .frame(AnchorSite::Vertex(0))
            .ok()
            .map(AnchorFrame::position),
    );
}

#[test]
fn quad_and_triangle_cells_keep_their_authored_row_major_addresses() {
    assert_eq!(QuadCell::new(2, 3).row(), 2);
    assert_eq!(QuadCell::new(2, 3).column(), 3);
    assert_eq!(TriangleCell::new(1, 2).row(), 1);
    assert_eq!(TriangleCell::new(1, 2).column(), 2);
    assert_eq!(
        TriangleCell::new(1, 2).orientation(),
        TriangleCellOrientation::Downward,
    );
}

#[test]
fn example_local_hex_box_and_no_fold_providers_use_the_same_public_trait() {
    let mut app = app();
    let hex = spawn_sheet(&mut app, HexFlower);
    let box_net = spawn_sheet(&mut app, BoxNet);
    let loose = spawn_sheet(&mut app, LooseTiles);

    let hex_members = member_entities(&mut app, hex);
    let box_members = member_entities(&mut app, box_net);
    assert_eq!(hex_members.len(), usize::from(HEX_PETALS) + 1);
    assert_eq!(retained_groups(&app, hex, &HexPetalSelection).len(), 1);
    assert_eq!(
        retained_groups(&app, hex, &HexPetalSelection)[0].len(),
        usize::from(HEX_PETALS),
    );

    let box_groups = retained_groups(&app, box_net, &BoxSideSelection);
    assert_eq!(box_groups.len(), 2);
    assert_eq!(box_groups[0].len(), 4);
    assert_eq!(box_groups[1], vec![box_members[5]]);

    assert_eq!(member_entities(&mut app, loose).len(), 3);
    assert!(
        RetainedProviderKnowledge::for_arrangement(app.world(), loose)
            .is_ok_and(|retained| retained.groups(&HexPetalSelection).is_err()),
    );
}

// Fold recipes and hinge calibration.

const TILT: f32 = core::f32::consts::FRAC_PI_4;
const FOLD_OFFSET: f32 = core::f32::consts::FRAC_PI_2;
const POSITIVE_CLEARANCE: Displacement = Displacement::new(0.0, 0.0, 0.5);
const NEGATIVE_CLEARANCE: Displacement = Displacement::new(0.0, 0.0, -0.25);

/// Selection vocabulary of the tilted chain used by the recipe tests.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ChainSelection;

/// A chain whose connections tilt away from zero and clear asymmetrically.
///
/// Provider sheets author centered clearance everywhere, so directional pivot
/// coverage needs hand-authored [`ArrangementConnection`] values.
#[derive(Clone, Copy)]
struct TiltedChain {
    length:  u8,
    capable: bool,
}

impl TiltedChain {
    const fn capable(length: u8) -> Self {
        Self {
            length,
            capable: true,
        }
    }

    const fn without_capability(length: u8) -> Self {
        Self {
            length,
            capable: false,
        }
    }
}

impl ArrangementProvider for TiltedChain {
    type FoldGroupSelection = ChainSelection;
    type Member = u8;

    fn members(&self) -> impl Iterator<Item = Self::Member> { 0..self.length }

    fn generate_plan(
        &self,
        members: &ArrangementMemberEntities<Self::Member>,
    ) -> Result<ArrangementPlan<Self::FoldGroupSelection>, ArrangementError> {
        let mut connections = Vec::new();
        let mut creases = Vec::new();
        for member in 1..self.length {
            connections.push(tilted_connection(members, member, member - 1));
            creases.push(members.entity(&member)?);
        }
        let group = FoldGroup::try_from_iter(creases).map_err(ArrangementError::provider)?;
        let groups = FoldGroups::from(group);
        let plan = ArrangementPlan::try_new(members, connections)?;
        if !self.capable {
            return plan.with_fold_groups(ChainSelection, groups);
        }
        let clearance = self.provide(&ChainSelection, &groups, plan.connections())?;

        plan.with_fold_groups(ChainSelection, groups)?
            .with_capability(ChainSelection, clearance)
    }
}

impl Provides<WindingClearance> for TiltedChain {
    fn provide(
        &self,
        _selection: &ChainSelection,
        groups: &FoldGroups,
        connections: &[ArrangementConnection],
    ) -> Result<WindingClearance, ArrangementError> {
        WindingClearance::try_new(
            groups,
            connections
                .iter()
                .map(|connection| (connection.member_entity, WINDING_STEP)),
        )
        .map_err(ArrangementError::provider)
    }
}

fn tilted_connection(
    members: &ArrangementMemberEntities<u8>,
    source: u8,
    target: u8,
) -> ArrangementConnection {
    ArrangementConnection {
        hinge_clearance: HingeClearance::new(POSITIVE_CLEARANCE, NEGATIVE_CLEARANCE),
        base_angle: angle(TILT),
        ..connection(members, source, target)
    }
}

fn angle(radians: f32) -> Angle {
    let authored = Angle::from_radians(radians);
    assert!(authored.is_ok(), "test angle must be representable");

    authored.unwrap_or_default()
}

fn hinge_of(app: &App, member: Entity) -> Hinge {
    let hinge = app.world().get::<Hinge>(member).copied();
    assert!(hinge.is_some(), "member {member:?} carries no hinge");

    hinge.unwrap_or_else(baseline_placeholder)
}

fn baseline_placeholder() -> Hinge {
    let hinge = Hinge::try_new(
        Edge {
            start: AnchorSite::Vertex(0),
            end:   AnchorSite::Vertex(1),
        },
        Angle::default(),
        Angle::default(),
        Displacement::default(),
    );
    assert!(
        hinge.is_ok(),
        "baseline hinge with default angles must be constructible"
    );

    hinge.unwrap_or_else(|_| unreachable_hinge())
}

fn unreachable_hinge() -> Hinge {
    unreachable!("baseline hinge construction is already asserted to succeed")
}

fn chain_creases(app: &mut App, arrangement: Entity) -> Vec<Entity> {
    let mut creases = member_entities(app, arrangement);
    creases.retain(|member| app.world().get::<Hinge>(*member).is_some());
    creases
}

fn apply<S, R>(app: &mut App, arrangement: Entity, selection: S, recipe: R)
where
    S: Eq + Hash + Debug + Send + Sync + 'static,
    R: FoldRecipe + Send + Sync + 'static,
{
    {
        let mut commands = app.world_mut().commands();
        commands.apply_fold_recipe(arrangement, selection, recipe);
    }
    app.world_mut().flush();
}

#[test]
fn baseline_hinges_rest_at_the_provider_angle_with_no_pivot() {
    let mut app = app();
    let arrangement = spawn_sheet(&mut app, TiltedChain::capable(3));
    let creases = chain_creases(&mut app, arrangement);

    assert_eq!(
        creases.len(),
        2,
        "one crease per connection, none for the root"
    );
    for crease in creases {
        let hinge = hinge_of(&app, crease);
        assert_eq!(hinge.base_angle(), angle(TILT));
        assert_eq!(hinge.folded_angle(), angle(TILT));
        assert_eq!(hinge.pivot_offset(), Displacement::default());
    }
}

#[test]
fn a_capability_free_recipe_applies_where_a_provider_capability_is_missing() {
    let mut app = app();
    let arrangement = spawn_sheet(&mut app, TiltedChain::without_capability(3));
    let knowledge = valid(
        "retained knowledge for a capability-free tilted chain",
        RetainedProviderKnowledge::for_arrangement(app.world(), arrangement),
    );

    assert!(NoCapability::retrieve(&knowledge, &ChainSelection).is_ok());
    assert!(matches!(
        WindingClearance::retrieve(&knowledge, &ChainSelection),
        Err(ArrangementError::MissingCapability { capability, .. })
            if capability.ends_with("WindingClearance"),
    ));

    apply(&mut app, arrangement, ChainSelection, Accordion::default());

    for crease in chain_creases(&mut app, arrangement) {
        assert_ne!(hinge_of(&app, crease).folded_angle(), angle(TILT));
    }
}

#[test]
fn a_recipe_needing_an_unfiled_capability_writes_no_hinge() {
    let mut app = app();
    let arrangement = spawn_sheet(&mut app, TiltedChain::without_capability(3));

    apply(
        &mut app,
        arrangement,
        ChainSelection,
        Wrap {
            fold_offset: angle(FOLD_OFFSET),
        },
    );

    for crease in chain_creases(&mut app, arrangement) {
        assert_eq!(hinge_of(&app, crease).folded_angle(), angle(TILT));
    }
}

#[test]
fn both_clearance_directions_reach_the_hinges_as_authored() {
    for (offset, expected_pivot) in [
        (FOLD_OFFSET, POSITIVE_CLEARANCE),
        (-FOLD_OFFSET, NEGATIVE_CLEARANCE),
    ] {
        let mut app = app();
        let arrangement = spawn_sheet(&mut app, TiltedChain::capable(3));

        apply(
            &mut app,
            arrangement,
            ChainSelection,
            Coil {
                fold_offset: angle(offset),
            },
        );

        for crease in chain_creases(&mut app, arrangement) {
            let hinge = hinge_of(&app, crease);
            assert_eq!(hinge.base_angle(), angle(TILT));
            assert_eq!(hinge.folded_angle(), angle(TILT + offset));
            assert_eq!(hinge.pivot_offset(), expected_pivot);
        }
    }
}

#[test]
fn a_recipe_replaces_endpoints_only_at_the_shared_base() {
    let mut app = app();
    let arrangement = spawn_sheet(&mut app, TiltedChain::capable(3));

    apply(
        &mut app,
        arrangement,
        ChainSelection,
        Coil {
            fold_offset: angle(FOLD_OFFSET),
        },
    );
    let folded = chain_creases(&mut app, arrangement)
        .into_iter()
        .map(|crease| hinge_of(&app, crease))
        .collect::<Vec<_>>();
    apply(
        &mut app,
        arrangement,
        ChainSelection,
        Coil {
            fold_offset: angle(-FOLD_OFFSET),
        },
    );

    let after = chain_creases(&mut app, arrangement)
        .into_iter()
        .map(|crease| hinge_of(&app, crease))
        .collect::<Vec<_>>();
    assert_eq!(after, folded, "an away-from-base recipe changes no hinge");
}

#[test]
fn a_member_that_disappeared_before_the_recipe_applies_is_skipped() {
    let mut app = app();
    let arrangement = spawn_sheet(&mut app, TiltedChain::capable(4));
    let creases = chain_creases(&mut app, arrangement);
    let (absent, surviving) = creases.split_at(1);
    let surviving = surviving.to_vec();
    app.world_mut().despawn(absent[0]);

    apply(
        &mut app,
        arrangement,
        ChainSelection,
        Coil {
            fold_offset: angle(FOLD_OFFSET),
        },
    );

    for crease in surviving {
        assert_eq!(
            hinge_of(&app, crease).folded_angle(),
            angle(TILT + FOLD_OFFSET)
        );
    }
}

#[test]
fn a_crease_free_sheet_rejects_a_recipe_and_writes_no_hinge() {
    let mut app = app();
    let arrangement = spawn_sheet(&mut app, QuadSheet::new(1, 1));
    let knowledge = valid(
        "retained knowledge for a crease-free quad sheet",
        RetainedProviderKnowledge::for_arrangement(app.world(), arrangement),
    );

    assert!(matches!(
        knowledge.groups(&QuadFoldGroupSelection::Rows),
        Err(ArrangementError::UnknownFoldGroupSelection { .. }),
    ));

    apply(
        &mut app,
        arrangement,
        QuadFoldGroupSelection::Rows,
        Accordion::default(),
    );

    assert!(chain_creases(&mut app, arrangement).is_empty());
}

#[test]
fn both_construction_routes_accept_a_recipe_in_the_same_command_batch() {
    let mut app = app();
    let bound_roots = (0..3)
        .map(|_| app.world_mut().spawn_empty().id())
        .collect::<Vec<_>>();
    let arrangements = {
        let mut commands = app.world_mut().commands();
        let spawned = commands
            .spawn_arrangement(TiltedChain::capable(3), |_| ())
            .unwrap_or(Entity::PLACEHOLDER);
        commands.apply_fold_recipe(
            spawned,
            ChainSelection,
            Coil {
                fold_offset: angle(FOLD_OFFSET),
            },
        );
        let bound = commands
            .spawn_arrangement_from_members(TiltedChain::capable(3), |member| {
                bound_roots
                    .get(usize::from(*member))
                    .map_or(MemberBinding::Missing, |entity| {
                        MemberBinding::Bound(*entity)
                    })
            })
            .unwrap_or(Entity::PLACEHOLDER);
        commands.apply_fold_recipe(
            bound,
            ChainSelection,
            Coil {
                fold_offset: angle(FOLD_OFFSET),
            },
        );
        [spawned, bound]
    };
    app.world_mut().flush();

    for arrangement in arrangements {
        let creases = chain_creases(&mut app, arrangement);
        assert_eq!(creases.len(), 2);
        for crease in creases {
            assert_eq!(
                hinge_of(&app, crease).folded_angle(),
                angle(TILT + FOLD_OFFSET),
            );
        }
    }
}

fn raw_connection(member_entity: Entity, target: Entity, base: f32) -> ArrangementConnection {
    ArrangementConnection {
        member_entity,
        anchored_to: AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
        member_edge: Edge {
            start: AnchorSite::Vertex(0),
            end:   AnchorSite::Vertex(1),
        },
        base_angle: angle(base),
        hinge_clearance: HingeClearance::new(POSITIVE_CLEARANCE, NEGATIVE_CLEARANCE),
    }
}

fn spawned_entities(app: &mut App, count: usize) -> Vec<Entity> {
    (0..count)
        .map(|_| app.world_mut().spawn_empty().id())
        .collect()
}

/// Unwraps a fixture, naming what was rejected and why on failure.
fn valid<T, E: Debug>(fixture: &str, result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("test fixture {fixture} was rejected: {error:?}"),
    }
}

/// Builds a fixture group, naming the rejected members on failure.
fn fold_group(members: &[Entity]) -> FoldGroup {
    match FoldGroup::try_from_iter(members.iter().copied()) {
        Ok(group) => group,
        Err(error) => panic!("test fixture group {members:?} was rejected: {error:?}"),
    }
}

fn one_group(members: &[Entity]) -> FoldGroups { FoldGroups::from(fold_group(members)) }

fn grouped(first: &[Entity], second: &[Entity]) -> FoldGroups {
    FoldGroups::new(fold_group(first), [fold_group(second)])
}

#[test]
fn accordion_alternates_direction_with_outer_group_parity() {
    let mut app = app();
    let members = spawned_entities(&mut app, 3);
    let connections = [
        raw_connection(members[1], members[0], TILT),
        raw_connection(members[2], members[1], TILT),
    ];
    let groups = grouped(&members[1..2], &members[2..3]);

    let assignments = Accordion {
        fold_offset: angle(FOLD_OFFSET),
    }
    .fold_assignments(&groups, &connections, &NoCapability);

    assert_eq!(
        assignments,
        Ok(vec![
            FoldAssignment {
                member_entity: members[1],
                folded_angle:  angle(TILT + FOLD_OFFSET),
                pivot_offset:  POSITIVE_CLEARANCE,
            },
            FoldAssignment {
                member_entity: members[2],
                folded_angle:  angle(TILT - FOLD_OFFSET),
                pivot_offset:  NEGATIVE_CLEARANCE,
            },
        ]),
    );
}

#[test]
fn accordion_reports_a_member_reached_by_two_opposite_groups() {
    let mut app = app();
    let members = spawned_entities(&mut app, 2);
    let connections = [raw_connection(members[1], members[0], TILT)];
    let groups = grouped(&members[1..2], &members[1..2]);

    assert_eq!(
        Accordion {
            fold_offset: angle(FOLD_OFFSET),
        }
        .fold_assignments(&groups, &connections, &NoCapability),
        Err(FoldAuthorError::ConflictingAccordionDirections {
            member_entity:     members[1],
            first_group:       0,
            conflicting_group: 1,
        }),
    );
}

#[test]
fn coil_and_wrap_assign_a_repeated_member_once_in_first_occurrence_order() {
    let mut app = app();
    let members = spawned_entities(&mut app, 4);
    let connections = [
        raw_connection(members[1], members[0], TILT),
        raw_connection(members[2], members[1], TILT),
        raw_connection(members[3], members[2], TILT),
    ];
    let groups = grouped(&members[1..3], &members[2..4]);
    let clearance = valid(
        "winding clearance for the two overlapping accordion groups",
        WindingClearance::try_new(
            &groups,
            members[1..4].iter().map(|member| (*member, WINDING_STEP)),
        ),
    );

    let coiled = Coil {
        fold_offset: angle(FOLD_OFFSET),
    }
    .fold_assignments(&groups, &connections, &NoCapability);
    let wrapped = Wrap {
        fold_offset: angle(FOLD_OFFSET),
    }
    .fold_assignments(&groups, &connections, &clearance);

    let order = |assignments: &Result<Vec<FoldAssignment>, FoldAuthorError>| {
        assignments.as_ref().map_or_else(
            |_| Vec::new(),
            |assignments| {
                assignments
                    .iter()
                    .map(|assignment| assignment.member_entity)
                    .collect::<Vec<_>>()
            },
        )
    };
    assert_eq!(order(&coiled), members[1..4].to_vec());
    assert_eq!(order(&wrapped), members[1..4].to_vec());
    // Each wound layer stacks one clearance further out than the last.
    assert_eq!(
        wrapped.map_or_else(
            |_| Vec::new(),
            |assignments| assignments
                .iter()
                .map(|assignment| assignment.pivot_offset)
                .collect::<Vec<_>>()
        ),
        vec![WINDING_STEP, WINDING_STEP * 2.0, WINDING_STEP * 3.0],
    );
}

#[test]
fn wrap_called_directly_with_incomplete_winding_input_names_the_member() {
    let mut app = app();
    let members = spawned_entities(&mut app, 3);
    let connections = [
        raw_connection(members[1], members[0], TILT),
        raw_connection(members[2], members[1], TILT),
    ];
    let covered = one_group(&members[1..2]);
    let selected = one_group(&members[1..3]);
    let clearance = valid(
        "winding clearance covering only the first member",
        WindingClearance::try_new(&covered, [(members[1], WINDING_STEP)]),
    );

    assert_eq!(
        Wrap {
            fold_offset: angle(FOLD_OFFSET),
        }
        .fold_assignments(&selected, &connections, &clearance),
        Err(FoldAuthorError::MissingWindingClearance {
            member_entity: members[2],
        }),
    );
}

#[test]
fn every_built_in_reports_an_unrepresentable_endpoint() {
    let mut app = app();
    let members = spawned_entities(&mut app, 2);
    let connections = [raw_connection(members[1], members[0], f32::MAX)];
    let groups = one_group(&members[1..2]);
    let clearance = valid(
        "winding clearance for the overflowing endpoint group",
        WindingClearance::try_new(&groups, [(members[1], WINDING_STEP)]),
    );
    let overflowing = angle(f32::MAX);
    let expected = Err(FoldAuthorError::AngleOverflow {
        member_entity: members[1],
    });

    assert_eq!(
        Accordion {
            fold_offset: overflowing,
        }
        .fold_assignments(&groups, &connections, &NoCapability),
        expected,
    );
    assert_eq!(
        Coil {
            fold_offset: overflowing,
        }
        .fold_assignments(&groups, &connections, &NoCapability),
        expected,
    );
    assert_eq!(
        Wrap {
            fold_offset: overflowing,
        }
        .fold_assignments(&groups, &connections, &clearance),
        expected,
    );
}

/// Failure a downstream recipe defines for itself.
#[derive(Debug, thiserror::Error)]
#[error("downstream recipe refused to fold")]
struct DownstreamFailure;

/// How one downstream recipe misbehaves, one per assignment-coverage error.
#[derive(Clone, Copy)]
enum DownstreamBehavior {
    Fold,
    Fail,
    OmitFirst,
    DuplicateFirst,
    Foreign,
    NonFinitePivot,
}

/// Downstream recipe proving the trait extends outside this crate.
#[derive(Clone, Copy)]
struct DownstreamRecipe(DownstreamBehavior);

impl FoldRecipe for DownstreamRecipe {
    type Error = DownstreamFailure;
    type RequiredCapability = NoCapability;

    fn fold_assignments(
        &self,
        groups: &FoldGroups,
        connections: &[ArrangementConnection],
        _: &Self::RequiredCapability,
    ) -> Result<Vec<FoldAssignment>, Self::Error> {
        if matches!(self.0, DownstreamBehavior::Fail) {
            return Err(DownstreamFailure);
        }
        let mut assignments = groups
            .iter()
            .flat_map(|group| group.connections(connections))
            .map(|connection| FoldAssignment {
                member_entity: connection.member_entity,
                folded_angle:  angle(TILT + FOLD_OFFSET),
                pivot_offset:  POSITIVE_CLEARANCE,
            })
            .collect::<Vec<_>>();
        match self.0 {
            DownstreamBehavior::Fold | DownstreamBehavior::Fail => {},
            DownstreamBehavior::OmitFirst => {
                assignments.remove(0);
            },
            DownstreamBehavior::DuplicateFirst => {
                let repeated = assignments[0];
                assignments.push(repeated);
            },
            DownstreamBehavior::Foreign => {
                assignments[0].member_entity = Entity::PLACEHOLDER;
            },
            DownstreamBehavior::NonFinitePivot => {
                assignments[0].pivot_offset = Displacement::new(f32::NAN, 0.0, 0.0);
            },
        }

        Ok(assignments)
    }
}

#[test]
fn a_downstream_recipe_folds_and_its_own_error_changes_no_hinge() {
    let mut app = app();
    let folded = spawn_sheet(&mut app, TiltedChain::capable(3));
    let refused = spawn_sheet(&mut app, TiltedChain::capable(3));

    apply(
        &mut app,
        folded,
        ChainSelection,
        DownstreamRecipe(DownstreamBehavior::Fold),
    );
    apply(
        &mut app,
        refused,
        ChainSelection,
        DownstreamRecipe(DownstreamBehavior::Fail),
    );

    for crease in chain_creases(&mut app, folded) {
        assert_eq!(
            hinge_of(&app, crease).folded_angle(),
            angle(TILT + FOLD_OFFSET),
        );
    }
    for crease in chain_creases(&mut app, refused) {
        assert_eq!(hinge_of(&app, crease).folded_angle(), angle(TILT));
    }
}

#[test]
fn every_assignment_coverage_failure_leaves_every_hinge_unchanged() {
    for behavior in [
        DownstreamBehavior::OmitFirst,
        DownstreamBehavior::DuplicateFirst,
        DownstreamBehavior::Foreign,
        DownstreamBehavior::NonFinitePivot,
    ] {
        let mut app = app();
        let arrangement = spawn_sheet(&mut app, TiltedChain::capable(4));

        apply(
            &mut app,
            arrangement,
            ChainSelection,
            DownstreamRecipe(behavior),
        );

        for crease in chain_creases(&mut app, arrangement) {
            let hinge = hinge_of(&app, crease);
            assert_eq!(hinge.folded_angle(), angle(TILT));
            assert_eq!(hinge.pivot_offset(), Displacement::default());
        }
    }
}

#[test]
fn a_two_piece_sheet_treats_each_piece_as_a_first_crease() {
    let mut app = app();
    let accordion = spawn_sheet(&mut app, TriangleSheet::new(4, 1));
    let wrapped = spawn_sheet(&mut app, TriangleSheet::new(4, 1));

    apply(
        &mut app,
        accordion,
        TriangleFoldGroupSelection::Columns,
        Accordion::default(),
    );
    apply(
        &mut app,
        wrapped,
        TriangleFoldGroupSelection::Columns,
        Wrap {
            fold_offset: angle(FOLD_OFFSET),
        },
    );

    let folded = chain_creases(&mut app, accordion)
        .into_iter()
        .map(|crease| hinge_of(&app, crease).folded_angle().radians())
        .collect::<Vec<_>>();
    assert!(
        folded.iter().all(|radians| *radians > 0.0),
        "each piece starts its own parity run: {folded:?}",
    );

    let layers = chain_creases(&mut app, wrapped)
        .into_iter()
        .map(|crease| hinge_of(&app, crease).pivot_offset())
        .collect::<Vec<_>>();
    // Two pieces, so both creases are a first layer; a single piece would have
    // stacked the second crease twice as far out.
    assert_eq!(layers.len(), 2);
    assert!(
        layers.first() == layers.last() && layers.first() != Some(&Displacement::default()),
        "each piece stacks its own first layer: {layers:?}",
    );
}

#[test]
fn a_two_piece_sheet_restarts_every_row_group_that_is_its_own_piece() {
    let mut app = app();
    let accordion = spawn_sheet(&mut app, TriangleSheet::new(4, 1));
    let wrapped = spawn_sheet(&mut app, TriangleSheet::new(4, 1));

    apply(
        &mut app,
        accordion,
        TriangleFoldGroupSelection::Rows,
        Accordion::default(),
    );
    apply(
        &mut app,
        wrapped,
        TriangleFoldGroupSelection::Rows,
        Wrap {
            fold_offset: angle(FOLD_OFFSET),
        },
    );

    // Rows 0 and 2 are forest roots, so this selection is two single-member
    // groups rather than the one two-member group `Columns` produces.
    let groups = retained_groups(&app, accordion, &TriangleFoldGroupSelection::Rows);
    assert_eq!(
        groups.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![1, 1],
        "one group per hinged row",
    );

    let folded = groups
        .concat()
        .into_iter()
        .map(|crease| hinge_of(&app, crease).folded_angle().radians())
        .collect::<Vec<_>>();
    assert!(
        folded.iter().all(|radians| *radians > 0.0),
        "each piece restarts group parity: {folded:?}",
    );

    let layers = retained_groups(&app, wrapped, &TriangleFoldGroupSelection::Rows)
        .concat()
        .into_iter()
        .map(|crease| hinge_of(&app, crease).pivot_offset())
        .collect::<Vec<_>>();
    assert_eq!(layers.len(), 2);
    assert!(
        layers.first() == layers.last() && layers.first() != Some(&Displacement::default()),
        "each piece stacks its own first layer: {layers:?}",
    );
}

#[test]
fn one_connected_piece_alternates_and_stacks_across_all_of_its_groups() {
    let mut app = app();
    let alternated = spawn_sheet(&mut app, QuadSheet::new(2, 2));
    let stacked = spawn_sheet(&mut app, QuadSheet::new(2, 2));

    apply(
        &mut app,
        alternated,
        QuadFoldGroupSelection::Rows,
        Accordion::default(),
    );
    apply(
        &mut app,
        stacked,
        QuadFoldGroupSelection::Rows,
        Wrap {
            fold_offset: angle(FOLD_OFFSET),
        },
    );

    // Every cell of this sheet reaches every other one, so both row groups
    // belong to one piece and outer group position keeps running across them.
    let groups = retained_groups(&app, alternated, &QuadFoldGroupSelection::Rows);
    assert_eq!(
        groups.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![1, 2],
        "row zero contributes only its non-root cell",
    );
    for (position, group) in groups.iter().enumerate() {
        for crease in group {
            let folded = hinge_of(&app, *crease).folded_angle().radians();
            if position.is_multiple_of(2) {
                assert!(folded > 0.0, "group {position} folds forward: {folded}");
            } else {
                assert!(folded < 0.0, "group {position} reverses: {folded}");
            }
        }
    }

    let wound = retained_groups(&app, stacked, &QuadFoldGroupSelection::Rows).concat();
    assert_eq!(wound.len(), 3, "every non-root cell winds once");
    let mut layers = 0.0_f32;
    for crease in wound {
        layers += 1.0;
        let layer = winding_layer(&app, stacked, &QuadFoldGroupSelection::Rows, crease);
        assert_eq!(
            hinge_of(&app, crease).pivot_offset(),
            layer * layers,
            "crease {crease:?} stacks layer {layers}",
        );
    }
}

fn winding_layer<S>(app: &App, arrangement: Entity, selection: &S, member: Entity) -> Displacement
where
    S: Eq + Hash + Debug + Send + Sync + 'static,
{
    let retained = RetainedProviderKnowledge::for_arrangement(app.world(), arrangement);
    assert!(
        retained.is_ok(),
        "arrangement retained no provider knowledge"
    );
    let Ok(retained) = retained else {
        return Displacement::default();
    };

    let clearance = retained.capability::<S, WindingClearance>(selection);
    assert!(
        clearance.is_ok(),
        "{selection:?} retained no winding clearance"
    );
    let Ok(clearance) = clearance else {
        return Displacement::default();
    };

    clearance
        .clearance_for(member)
        .unwrap_or_else(|_| Displacement::default())
}

const HINGE_PIVOT_RADIUS: f32 = 0.25;
const POSE_EPSILON: f32 = 1e-4;
const QUARTER_TURN: f32 = core::f32::consts::FRAC_PI_2;

/// Geometry whose ordered edge runs from `-Y` to `+Y` in an unrotated frame.
fn hinge_geometry() -> ResolvedAnchorGeometry {
    let geometry = ResolvedAnchorGeometry::try_new(
        [
            (AnchorSite::Center, site_frame(Vec3::ZERO)),
            (AnchorSite::Vertex(0), site_frame(Vec3::NEG_Y)),
            (AnchorSite::Vertex(1), site_frame(Vec3::Y)),
        ],
        [hinge_edge()],
    );
    assert!(geometry.is_ok(), "hinge geometry must be constructible");

    geometry.unwrap_or_default()
}

fn site_frame(position: Vec3) -> AnchorFrame {
    let frame = AnchorFrame::try_new(Position::from(position), Orientation::default());
    assert!(frame.is_ok(), "anchor frame {position:?} must be finite");

    frame.unwrap_or_default()
}

const fn hinge_edge() -> Edge {
    Edge {
        start: AnchorSite::Vertex(0),
        end:   AnchorSite::Vertex(1),
    }
}

fn folded_hinge(base: f32, folded: f32, pivot: Displacement) -> Hinge {
    let hinge = Hinge::try_new(hinge_edge(), angle(base), angle(folded), pivot);
    assert!(
        hinge.is_ok(),
        "hinge {base}..{folded} must be constructible"
    );

    hinge.unwrap_or_else(|_| baseline_placeholder())
}

fn folding_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        TaskPoolPlugin::default(),
        AssetPlugin::default(),
        FoldPlugin,
    ))
    .insert_resource(Time::<Virtual>::default());
    // Warning capture installs a thread-local subscriber, so every system these
    // tests observe has to run on the calling thread.
    app.edit_schedule(Update, |schedule| {
        schedule.set_executor(SingleThreadedExecutor::new());
    });
    app.edit_schedule(PostUpdate, |schedule| {
        schedule.set_executor(SingleThreadedExecutor::new());
    });
    app
}

fn pose_of(app: &App, entity: Entity) -> AnchorPose {
    app.world()
        .get::<AnchorPose>(entity)
        .copied()
        .unwrap_or_default()
}

/// Plays every retained sequence to its folded endpoint.
fn play_to_folded(
    mut fold_commands: FoldCommands,
    sequences: Query<Entity, With<FoldSequencePlayback>>,
) {
    for sequence in &sequences {
        fold_commands.apply(
            sequence,
            SequenceOwner::NativePlayback,
            SequenceCommand::Play,
        );
    }
}

#[test]
fn a_folded_hinge_translates_by_its_pivot_turned_through_the_angle_delta() {
    let mut app = folding_app();
    let target = app.world_mut().spawn_empty().id();
    let member = app
        .world_mut()
        .spawn((
            hinge_geometry(),
            AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
            folded_hinge(
                0.0,
                QUARTER_TURN,
                Displacement::new(0.0, 0.0, HINGE_PIVOT_RADIUS),
            ),
        ))
        .id();
    app.world_mut().spawn(
        FoldSequenceBuilder::new(FoldTiming::new(
            Duration::from_secs(1),
            EaseFunction::Linear,
        ))
        .stage(FoldStage::from(member))
        .build(),
    );

    app.update();
    app.world_mut()
        .run_system_once(play_to_folded)
        .expect("playing every retained sequence never fails");
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .advance_by(Duration::from_secs(1));
    app.update();

    let pose = pose_of(&app, member);
    // The sequence rests at its folded endpoint, so the delta from the zero
    // base angle is a quarter turn about the edge axis, which is +Y in this
    // unrotated source frame. A quarter turn about +Y carries the pivot
    // (0, 0, r) onto (r, 0, 0), and the compensation is their difference.
    let expected = Vec3::new(-HINGE_PIVOT_RADIUS, 0.0, HINGE_PIVOT_RADIUS);
    let translation = pose.translation.into_inner();
    assert!(
        (translation - expected).length() <= POSE_EPSILON,
        "translation {translation:?}, expected {expected:?}",
    );
    let rotation = pose.rotation.into_inner();
    let expected_rotation = Quat::from_axis_angle(Vec3::Y, QUARTER_TURN);
    assert!(
        rotation.dot(expected_rotation).abs() >= 1.0 - POSE_EPSILON,
        "rotation {rotation:?}, expected {expected_rotation:?}",
    );
}

/// Counts warnings this crate raised while a schedule ran.
#[derive(Clone, Default)]
struct WarnCount(Arc<AtomicUsize>);

impl WarnCount {
    fn count(&self) -> usize { self.0.load(Ordering::Relaxed) }
}

impl<S: Subscriber> Layer<S> for WarnCount {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if *event.metadata().level() == Level::WARN
            && event.metadata().target().starts_with("hana_valence")
        {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Runs `schedule` on the calling thread with warnings counted.
fn run_counted(counter: &WarnCount, world: &mut World, schedule: &mut Schedule) {
    let subscriber = Registry::default().with(counter.clone());
    bevy::log::tracing::subscriber::with_default(subscriber, || schedule.run(world));
}

fn hinge_schedule() -> Schedule {
    let mut schedule = Schedule::default();
    schedule.set_executor(SingleThreadedExecutor::new());
    schedule.add_systems(hinge_to_pose);
    schedule
}

/// Turns `overwrite_poses` on for a single run of the shared schedule.
#[derive(Resource, Default)]
struct Contend(bool);

fn contending(contend: Res<Contend>) -> bool { contend.0 }

fn overwrite_poses(mut poses: Query<&mut AnchorPose>) {
    for mut pose in &mut poses {
        pose.translation = Displacement::new(1.0, 0.0, 0.0);
    }
}

/// One schedule instance, so its second run compares against the first run's
/// tick rather than against a fresh `last_run` that predates every change.
fn contended_schedule() -> Schedule {
    let mut schedule = Schedule::default();
    schedule.set_executor(SingleThreadedExecutor::new());
    schedule.add_systems((overwrite_poses.run_if(contending), hinge_to_pose).chain());
    schedule
}

#[test]
fn a_required_pose_stays_silent_and_an_earlier_same_frame_write_warns() {
    let counter = WarnCount::default();
    let mut world = World::new();
    world.init_resource::<Contend>();
    let target = world.spawn_empty().id();
    world.spawn((
        hinge_geometry(),
        AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
        folded_hinge(QUARTER_TURN, QUARTER_TURN, Displacement::default()),
    ));
    let mut schedule = contended_schedule();

    // `#[require(AnchorPose)]` adds the pose with the hinge, so its added and
    // changed ticks are the same one and materialization stays silent.
    run_counted(&counter, &mut world, &mut schedule);
    assert_eq!(
        counter.count(),
        0,
        "a freshly required pose is not a rewrite"
    );

    world.resource_mut::<Contend>().0 = true;
    run_counted(&counter, &mut world, &mut schedule);

    // The warning is `#[cfg(debug_assertions)]`, so a release run counts zero
    // for a reason that is not a regression.
    let expected = usize::from(cfg!(debug_assertions));
    assert_eq!(
        counter.count(),
        expected,
        "a pose written earlier in the same frame is overwritten once",
    );
}

#[test]
fn unfilled_geometry_stays_silent_while_an_authoring_gap_warns_once() {
    let counter = WarnCount::default();
    let mut world = World::new();
    let target = world.spawn_empty().id();
    let resting = folded_hinge(QUARTER_TURN, QUARTER_TURN, Displacement::default());
    world.spawn(resting);
    let without_relationship = world.spawn((hinge_geometry(), resting)).id();
    let mut schedule = hinge_schedule();

    run_counted(&counter, &mut world, &mut schedule);
    run_counted(&counter, &mut world, &mut schedule);

    // Geometry arrives a frame or more after materialization, so its absence
    // never warns; a missing relationship is an authoring gap and warns once.
    assert_eq!(counter.count(), 1, "one warning per misauthored entity");
    assert_eq!(
        world.get::<AnchorPose>(without_relationship).copied(),
        Some(AnchorPose::default()),
        "a skipped hinge leaves its pose in place",
    );

    world
        .entity_mut(without_relationship)
        .insert(AnchoredTo::new(
            target,
            AnchorSite::Center,
            AnchorSite::Center,
        ));
    run_counted(&counter, &mut world, &mut schedule);

    assert_eq!(counter.count(), 1, "repair raises no further warning");
    let rotation = world
        .get::<AnchorPose>(without_relationship)
        .map(|pose| pose.rotation.into_inner())
        .unwrap_or_default();
    let expected = Quat::from_axis_angle(Vec3::Y, QUARTER_TURN);
    assert!(
        rotation.dot(expected).abs() >= 1.0 - POSE_EPSILON,
        "rotation {rotation:?}, expected {expected:?}",
    );
}

#[test]
fn a_member_track_sampled_at_a_position_that_names_no_place_stays_inside_a_segment() {
    let mut world = World::new();
    let member = world.spawn_empty().id();
    let sequence = FoldSequenceBuilder::new(FoldTiming::new(
        Duration::from_secs(1),
        EaseFunction::Linear,
    ))
    .stage(FoldStage::from(member))
    .stage(FoldStage::from(member))
    .build();
    let [track] = sequence.tracks() else {
        unreachable!("one member authors one track");
    };

    let FoldMemberSample::Moving {
        segment,
        raw_progress,
    } = track.sample(f64::NAN)
    else {
        panic!("a position that names no place enters every segment");
    };
    assert_eq!(segment.stage_ordinal(), 1);
    assert_eq!(raw_progress, FoldSegmentProgress::ENTERED);
    assert_eq!(
        segment.raw_progress(f64::NEG_INFINITY),
        FoldSegmentProgress::ENTERED,
    );
    assert_eq!(
        segment.raw_progress(f64::INFINITY),
        FoldSegmentProgress::FULLY_TRAVELLED,
    );
}

// Retained fold playback: authoring, arbitration, easing, and reporting.

const FRACTION_EPSILON: f64 = 1e-5;
const HALFWAY: f64 = 0.5;
/// The same midpoint as a curve input, which curve knots take in `f32`.
const HALFWAY_INPUT: f32 = 0.5;
const SECOND: Duration = Duration::from_secs(1);
const THIRD: f64 = 1.0 / 3.0;

/// Spawns a hinged member anchored to `target` that travels a quarter turn.
fn quarter_turn_member(app: &mut App, target: Entity) -> Entity {
    app.world_mut()
        .spawn((
            hinge_geometry(),
            AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
            folded_hinge(0.0, QUARTER_TURN, Displacement::default()),
        ))
        .id()
}

/// Spawns one shared anchor target plus `count` hinged members on it.
fn quarter_turn_members(app: &mut App, count: usize) -> (Entity, Vec<Entity>) {
    let target = app.world_mut().spawn_empty().id();
    let members = (0..count)
        .map(|_| quarter_turn_member(app, target))
        .collect();
    (target, members)
}

/// Replaces this update's virtual delta and runs one update.
///
/// These apps run no `TimePlugin`, so `Time<Virtual>` keeps the delta of the
/// last advance until something replaces it. Every update in this section goes
/// through here, so an update that is not meant to move time advances
/// `Duration::ZERO` rather than replaying the previous delta.
fn advance(app: &mut App, delta: Duration) {
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .advance_by(delta);
    app.update();
}

fn issue_fold_command(
    input: In<(Entity, SequenceOwner, SequenceCommand)>,
    mut fold_commands: FoldCommands,
) -> SequenceCommandResponse {
    let (sequence, issuer, fold_command) = input.0;
    fold_commands.apply(sequence, issuer, fold_command)
}

/// Issues one shared command against a retained sequence.
fn command(
    app: &mut App,
    sequence: Entity,
    issuer: SequenceOwner,
    fold_command: SequenceCommand,
) -> SequenceCommandResponse {
    app.world_mut()
        .run_system_once_with(issue_fold_command, (sequence, issuer, fold_command))
        .expect("issuing a fold command never fails")
}

fn read_owner(input: In<Entity>, fold_commands: FoldCommands) -> SequenceOwnership {
    fold_commands.owner(input.0)
}

/// Returns who currently owns a retained sequence's local position.
fn owner_of(app: &mut App, sequence: Entity) -> SequenceOwner {
    match app
        .world_mut()
        .run_system_once_with(read_owner, sequence)
        .expect("reading a sequence owner never fails")
    {
        SequenceOwnership::Retained(owner) => owner,
        SequenceOwnership::NoRetainedSequence => {
            panic!("a retained fold sequence always reports an owner")
        },
    }
}

/// Returns what a retained sequence holds for one member this update.
fn fraction_of(app: &App, sequence: Entity, member: Entity) -> FoldMemberFraction {
    app.world()
        .get::<FoldSequencePlayback>(sequence)
        .map_or(FoldMemberFraction::Untracked, |playback| {
            playback.member_fraction(member)
        })
}

/// Returns the eased fraction of a member that resolved one.
fn eased_of(app: &App, sequence: Entity, member: Entity) -> f64 {
    match fraction_of(app, sequence, member) {
        FoldMemberFraction::Eased(fraction) => fraction.value(),
        other => panic!("member {member:?} resolved no fraction: {other:?}"),
    }
}

/// Returns a retained sequence's current raw normalized position.
fn position_of(app: &App, sequence: Entity) -> f64 {
    app.world()
        .get::<FoldSequencePlayback>(sequence)
        .map_or(f64::NAN, FoldSequencePlayback::normalized_position)
}

fn assert_close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= FRACTION_EPSILON,
        "{what}: {actual}, expected {expected}",
    );
}

/// Builds a forward movement that names one absolute position.
fn movement(position: f64) -> SequenceMovement {
    movement_toward(position, SequenceDirection::Forward)
}

/// Builds a movement that names one absolute position and direction.
fn movement_toward(position: f64, direction: SequenceDirection) -> SequenceMovement {
    let position = valid(
        "movement position",
        #[expect(
            clippy::cast_possible_truncation,
            reason = "normalized test positions are exactly representable"
        )]
        SequencePosition::try_new(position as f32),
    );
    valid(
        "producer movement",
        SequenceMovement::try_new(position, direction, 0, RangeCrossings::NONE),
    )
}

fn easing_input(value: f32) -> EasingInput { valid("easing input", EasingInput::try_new(value)) }

fn easing_output(value: f32) -> EasingOutput {
    valid("easing output", EasingOutput::try_new(value))
}

fn easing_slope(value: f32) -> EasingSlope { valid("easing slope", EasingSlope::try_new(value)) }

/// A piecewise-linear curve that maps `0.5` onto `0.25` and stays bounded and
/// monotonic, so it eases every scope.
fn bounded_curve() -> EasingCurve {
    valid(
        "bounded lookup curve",
        EasingCurve::builder()
            .linear()
            .knot(easing_input(0.0), easing_output(0.0))
            .knot(easing_input(HALFWAY_INPUT), easing_output(0.25))
            .knot(easing_input(1.0), easing_output(1.0))
            .try_build(),
    )
}

/// The same curve's mapping, for computing expected output in a test.
fn bounded_curve_at(progress: f64) -> f64 {
    if progress <= HALFWAY {
        progress * HALFWAY
    } else {
        (progress - HALFWAY).mul_add(1.5, 0.25)
    }
}

/// A curve that leaves `0..=1`, so it eases one stage's output and no wider
/// scope.
fn overshooting_curve() -> EasingCurve {
    valid(
        "overshooting lookup curve",
        EasingCurve::builder()
            .linear()
            .knot(easing_input(0.0), easing_output(0.0))
            .knot(easing_input(HALFWAY_INPUT), easing_output(1.5))
            .knot(easing_input(1.0), easing_output(1.0))
            .try_build(),
    )
}

/// A curve whose interior cubic segment overflows `f32`, so sampling it names
/// no output at all.
///
/// Both interior knots hold `f32::MAX` and their facing tangents point away
/// from each other, so every interior point of that segment adds a positive
/// tangent term to a value already at the maximum.
fn overflowing_curve() -> EasingCurve {
    let knot = |input: f32, output: f32| {
        EasingKnot::new(
            easing_input(input),
            easing_output(output),
            EasingInterpolation::Cubic,
        )
    };
    let slopes = |incoming: f32, outgoing: f32| {
        EasingSlopes::default()
            .with_incoming(easing_slope(incoming))
            .with_outgoing(easing_slope(outgoing))
    };
    valid(
        "overflowing lookup curve",
        EasingCurve::try_new([
            knot(0.0, 0.0),
            knot(0.25, f32::MAX).with_slopes(slopes(0.0, f32::MAX)),
            knot(0.75, f32::MAX).with_slopes(slopes(-f32::MAX, 0.0)),
            knot(1.0, 1.0),
        ]),
    )
}

/// Collects the messages and fields of every warning this crate raised.
#[derive(Clone, Default)]
struct WarnLog(Arc<Mutex<Vec<String>>>);

impl WarnLog {
    /// Returns how many recorded warnings name `condition`.
    fn naming(&self, condition: &str) -> usize {
        self.0.lock().map_or(0, |warnings| {
            warnings
                .iter()
                .filter(|warning| warning.contains(condition))
                .count()
        })
    }

    fn total(&self) -> usize { self.0.lock().map_or(0, |warnings| warnings.len()) }
}

impl<S: Subscriber> Layer<S> for WarnLog {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if *event.metadata().level() != Level::WARN
            || !event.metadata().target().starts_with("hana_valence")
        {
            return;
        }
        let mut recorded = RecordedWarning(String::new());
        event.record(&mut recorded);
        if let Ok(mut warnings) = self.0.lock() {
            warnings.push(recorded.0);
        }
    }
}

/// One warning's message and every field it named, flattened for matching.
struct RecordedWarning(String);

impl Visit for RecordedWarning {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.0.push_str(field.name());
        self.0.push('=');
        write!(self.0, "{value:?} ").expect("writing to a String never fails");
    }
}

/// Advances `app` with this crate's warnings recorded into `warnings`.
fn logged_advance(warnings: &WarnLog, app: &mut App, delta: Duration) {
    let subscriber = Registry::default().with(warnings.clone());
    bevy::log::tracing::subscriber::with_default(subscriber, || advance(app, delta));
}

/// Warning text of each per-sequence and per-hinge report slot.
const HINGE_LESS_TRACKS: &str = "carry no Hinge";
const HINGE_POSE_UNAVAILABLE: &str = "hinge pose unavailable";
const INVALID_MOVEMENT: &str = "contradicted local position";
const NON_FINITE_EASING: &str = "non-finite output; the pose holds";
const REJECTED_MULTI_STAGE_CURVE: &str = "rejected for its multi-stage scope";
const REJECTED_STAGE_OUTPUT_CURVE: &str = "rejected as output easing";
const STALE_PRODUCER_SCOPE: &str = "no longer resolves against the authored stages";

#[test]
fn zero_duration_and_all_zero_sequences_snap_every_member_to_its_target() {
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 2);
    let snapping = valid(
        "snap group",
        FoldGroup::try_from_iter(members.iter().copied()),
    );
    let sequence = app
        .world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::snap())
                .stage(FoldStage::from(snapping))
                .build(),
        )
        .id();

    advance(&mut app, Duration::ZERO);

    // Every authored duration is zero, so the whole ledger sits at the start
    // and reaching a zero-extent segment completes it.
    assert_close(
        position_of(&app, sequence),
        0.0,
        "an all-zero ledger holds the sequence start",
    );
    for member in &members {
        assert_close(
            eased_of(&app, sequence, *member),
            1.0,
            "a zero-duration member snaps to its target",
        );
    }
    assert_ne!(
        pose_of(&app, members[0]),
        AnchorPose::default(),
        "a snapped member reaches its folded pose with no travel",
    );
}

#[test]
fn arbitrary_forward_and_backward_travel_agree_with_direct_sampling() {
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 1);
    let member = members[0];
    let sequence = app
        .world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::new(
                Duration::from_secs(4),
                EaseFunction::Linear,
            ))
            .stage(FoldStage::from(member))
            .build(),
        )
        .id();
    advance(&mut app, Duration::ZERO);

    command(
        &mut app,
        sequence,
        SequenceOwner::NativePlayback,
        SequenceCommand::Play,
    );
    advance(&mut app, SECOND);
    let quarter = eased_of(&app, sequence, member);
    advance(&mut app, SECOND);
    let half_forward = eased_of(&app, sequence, member);

    command(
        &mut app,
        sequence,
        SequenceOwner::NativePlayback,
        SequenceCommand::PlayBackward,
    );
    advance(&mut app, SECOND);
    let quarter_backward = eased_of(&app, sequence, member);

    assert_close(quarter, 0.25, "a quarter of the authored travel");
    assert_close(half_forward, HALFWAY, "half the authored travel");
    assert_close(
        quarter_backward,
        0.25,
        "backward travel reaches the same fraction the forward pass did",
    );

    // Direct sampling of the immutable track answers the same values in any
    // order, which is what history independence means.
    let Some(authored) = app.world().get::<FoldSequence>(sequence) else {
        panic!("the authored sequence stays on its entity");
    };
    let sampled = [0.75, 0.25, HALFWAY, 0.0, 1.0, 0.25];
    let mut moving_samples = 0_usize;
    for position in sampled {
        let sample = authored.sample_member(member, position);
        if position >= 1.0 {
            assert!(
                matches!(sample, FoldMemberSample::Resting { .. }),
                "the sequence end rests at the destination it reached: {sample:?}",
            );
            continue;
        }
        let FoldMemberSample::Moving { raw_progress, .. } = sample else {
            panic!("{position} lies inside the sole authored segment: {sample:?}");
        };
        moving_samples += 1;
        assert_close(
            f64::from(raw_progress.normalized()),
            position,
            "arbitrary sampling depends only on the sampled position",
        );
    }
    assert_eq!(
        moving_samples,
        sampled.len() - 1,
        "every sampled position short of the end reported its own progress",
    );
}

#[test]
fn a_custom_closure_subdivides_reorders_and_omits_the_groups_it_was_handed() {
    let mut app = app();
    let subdivided = spawn_sheet(
        &mut app,
        QuadSheet::new(2, 2).with_custom_fold_sequence(
            QuadFoldGroupSelection::Rows,
            |groups: &FoldGroups| {
                Ok(FoldSequenceBuilder::new(fold_timing())
                    .stages(
                        groups
                            .iter()
                            .flat_map(|group| group.iter().copied().map(FoldStage::from)),
                    )
                    .build())
            },
        ),
    );
    let handed = Arc::new(Mutex::new(Vec::<Vec<Entity>>::new()));
    let recorder = Arc::clone(&handed);
    let reordered = spawn_sheet(
        &mut app,
        QuadSheet::new(2, 2).with_custom_fold_sequence(
            QuadFoldGroupSelection::Rows,
            move |groups: &FoldGroups| {
                if let Ok(mut handed) = recorder.lock() {
                    handed.extend(
                        groups
                            .iter()
                            .map(|group| group.iter().copied().collect::<Vec<Entity>>()),
                    );
                }
                let mut ordered: Vec<FoldGroup> = groups.iter().cloned().collect();
                ordered.reverse();
                Ok(FoldSequenceBuilder::new(fold_timing())
                    .stages(ordered)
                    .build())
            },
        ),
    );
    let omitted = spawn_sheet(
        &mut app,
        QuadSheet::new(2, 2).with_custom_fold_sequence(
            QuadFoldGroupSelection::Rows,
            |groups: &FoldGroups| {
                Ok(FoldSequenceBuilder::new(fold_timing())
                    .stages(groups.iter().take(1).cloned())
                    .build())
            },
        ),
    );

    let staged = |arrangement: Entity| -> (usize, usize, Vec<Entity>) {
        let Some(authored) = app.world().get::<FoldSequence>(arrangement) else {
            panic!("the custom adapter authored no sequence on {arrangement:?}");
        };
        (
            authored.stages().len(),
            authored.tracks().len(),
            authored
                .stages()
                .iter()
                .flat_map(|stage| stage.group().iter().copied())
                .collect(),
        )
    };

    // The 2x2 row alternative retains two groups covering three non-root cells.
    let (subdivided_stages, subdivided_tracks, subdivided_members) = staged(subdivided);
    let (reordered_stages, reordered_tracks, reordered_members) = staged(reordered);
    let (omitted_stages, omitted_tracks, omitted_members) = staged(omitted);

    assert_eq!((subdivided_stages, subdivided_tracks), (3, 3));
    assert_eq!((reordered_stages, reordered_tracks), (2, 3));
    assert_eq!((omitted_stages, omitted_tracks), (1, 1));
    let Ok(handed_groups) = handed.lock() else {
        panic!("the custom adapter recorded the groups it was handed");
    };
    let handed_order: Vec<Entity> = handed_groups.iter().flatten().copied().collect();
    let reversed_order: Vec<Entity> = handed_groups.iter().rev().flatten().copied().collect();
    assert_ne!(
        handed_order, reversed_order,
        "the provider hands over more than one distinguishable group",
    );
    assert_eq!(
        reordered_members, reversed_order,
        "reversing the groups stages the last group the provider offered first",
    );
    assert!(
        omitted_members.len() < subdivided_members.len(),
        "an omitted group leaves its members out of the sequence entirely",
    );
}

#[test]
fn a_maximum_duration_stage_holds_its_partner_at_the_sequence_start() {
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 2);
    let sequence = FoldSequenceBuilder::new(FoldTiming::new(SECOND, EaseFunction::Linear))
        .stage(
            FoldStage::from(members[0])
                .override_timing(FoldTiming::new(Duration::MAX, EaseFunction::Linear)),
        )
        .stage(FoldStage::from(members[1]))
        .build();

    // The exact total keeps both authored durations instead of saturating.
    assert_eq!(
        sequence.total(),
        SequenceTime::sum(&[Duration::MAX, SECOND])
    );
    let spawned = app.world_mut().spawn(sequence).id();
    advance(&mut app, Duration::ZERO);

    // The second stage occupies a vanishing share of the exact total, so at the
    // maximum interior position it has not begun and its member rests at base.
    let Some(authored) = app.world().get::<FoldSequence>(spawned) else {
        panic!("the authored sequence stays on its entity");
    };
    assert!(matches!(
        authored.sample_member(members[1], 0.999_999),
        FoldMemberSample::Resting {
            target: FoldTarget::BASE,
        }
    ));
    assert!(matches!(
        authored.sample_member(members[1], 1.0),
        FoldMemberSample::Resting {
            target: FoldTarget::FOLDED,
        }
    ));
    assert_close(
        eased_of(&app, spawned, members[1]),
        0.0,
        "the partner of a maximum-duration stage starts at base",
    );
}

#[test]
fn a_provider_wrapped_twice_aborts_the_spawn_with_no_scene_and_no_sequence() {
    let mut app = app();
    let arrangements_before = arrangement_count(&mut app);
    let entities_before = app.world().iter_entities().count();
    let scenes = Cell::new(0);
    let outcome = {
        let mut commands = app.world_mut().commands();
        commands.spawn_arrangement(
            QuadSheet::new(2, 2)
                .with_fold_sequence(QuadFoldGroupSelection::Rows, fold_timing())
                .with_custom_fold_sequence(QuadFoldGroupSelection::Rows, |groups: &FoldGroups| {
                    Ok(FoldSequenceBuilder::new(fold_timing())
                        .stage(FoldGroup::combine(groups.iter())?)
                        .build())
                }),
            |_| {
                scenes.set(scenes.get() + 1);
            },
        )
    };
    app.update();

    let Err(ArrangementError::DuplicateAuthoredFoldSequence) = outcome else {
        panic!("a second authored sequence must be rejected: {outcome:?}");
    };
    assert_eq!(scenes.get(), 0, "an aborted spawn builds no scene");
    assert_eq!(arrangement_count(&mut app), arrangements_before);
    assert_eq!(authored_sequence_count(&mut app), 0);
    assert_eq!(
        app.world().iter_entities().count(),
        entities_before,
        "an aborted spawn releases every reserved member entity",
    );
}

#[test]
fn a_custom_closure_tracking_an_entity_outside_the_plan_names_it_and_writes_nothing() {
    let mut app = app();
    let outside = app.world_mut().spawn_empty().id();
    let arrangements_before = arrangement_count(&mut app);
    let scenes = Cell::new(0);
    let outcome = {
        let mut commands = app.world_mut().commands();
        commands.spawn_arrangement(
            QuadSheet::new(2, 2).with_custom_fold_sequence(
                QuadFoldGroupSelection::Rows,
                move |_: &FoldGroups| {
                    Ok(FoldSequenceBuilder::new(fold_timing())
                        .stage(FoldGroup::from(outside))
                        .build())
                },
            ),
            |_| {
                scenes.set(scenes.get() + 1);
            },
        )
    };
    app.update();

    let Err(ArrangementError::ForeignFoldGroupMember { member_entity }) = outcome else {
        panic!("a member outside the plan must be named exactly: {outcome:?}");
    };
    assert_eq!(member_entity, outside);
    assert_eq!(scenes.get(), 0, "an aborted spawn builds no scene");
    assert_eq!(arrangement_count(&mut app), arrangements_before);
    assert_eq!(authored_sequence_count(&mut app), 0);
}

/// Three equal stages of one member, each carrying it one third further.
///
/// The authored easing is quadratic, so composed and replaced external curves
/// produce different output at the same position, and every stage has the same
/// extent, so a scope's normalized range is exact.
fn thirds_sequence(member: Entity) -> FoldSequence {
    let stage = |ordinal: u32| {
        let target = valid(
            "thirds target",
            #[expect(
                clippy::cast_precision_loss,
                reason = "three stage ordinals are exactly representable"
            )]
            FoldTarget::try_new((ordinal + 1) as f32 / 3.0),
        );
        valid(
            "thirds stage",
            FoldStage::from(member).with_member_target(member, target),
        )
    };
    FoldSequenceBuilder::new(FoldTiming::new(SECOND, EaseFunction::QuadraticIn))
        .stages([stage(0), stage(1), stage(2)])
        .build()
}

/// The fraction [`thirds_sequence`] resolves at `position` under its own
/// authored easing alone.
fn authored_thirds_fraction(position: f64) -> f64 {
    let stage = (position * 3.0).floor().clamp(0.0, 2.0);
    let raw = position.mul_add(3.0, -stage).clamp(0.0, 1.0);
    stage.mul_add(THIRD, raw * raw * THIRD)
}

/// Which authored extent a producer selects, named before the sequence it
/// selects from exists.
///
/// Every `FoldSequence` carries its own stage-description revision, so a
/// `SequenceStageId` is only valid against the sequence it was read from. This
/// names the extent instead and resolves it against whichever sequence is under
/// test.
#[derive(Clone, Copy, Debug)]
enum ThirdsScope {
    WholeSequence,
    FirstTwoStages,
    MiddleStage,
}

impl ThirdsScope {
    /// Resolves this extent against one live sequence's own stage identities.
    fn resolve(self, app: &App, sequence: Entity) -> SequenceScope {
        match self {
            Self::WholeSequence => SequenceScope::WholeSequence,
            Self::FirstTwoStages => SequenceScope::StageRange {
                first: stage_id(app, sequence, 0),
                last:  stage_id(app, sequence, 1),
            },
            Self::MiddleStage => SequenceScope::Stage(stage_id(app, sequence, 1)),
        }
    }

    /// Returns the normalized start and end this extent selects.
    fn extent(self) -> (f64, f64) {
        match self {
            Self::WholeSequence => (0.0, 1.0),
            Self::FirstTwoStages => (0.0, 2.0 * THIRD),
            Self::MiddleStage => (THIRD, 2.0 * THIRD),
        }
    }
}

/// Which external curve a producer publishes.
#[derive(Clone, Copy, Debug)]
enum ThirdsCurve {
    StockCubic,
    BoundedLookup,
}

impl ThirdsCurve {
    /// Returns this curve as the producer's immutable easing value.
    fn resolve(self) -> Easing {
        match self {
            Self::StockCubic => Easing::from(EaseFunction::CubicIn),
            Self::BoundedLookup => Easing::from(bounded_curve()),
        }
    }

    /// Returns the output this curve produces for `progress`.
    fn at(self, progress: f64) -> f64 {
        match self {
            Self::StockCubic => progress * progress * progress,
            Self::BoundedLookup => bounded_curve_at(progress),
        }
    }
}

/// How a producer eases the extent it selected, named before its curve exists.
#[derive(Clone, Copy, Debug)]
enum ThirdsEasing {
    Authored,
    ReplacedBy(ThirdsCurve),
    ComposedWith(ThirdsCurve),
}

impl ThirdsEasing {
    /// Resolves this choice to its immutable producer easing.
    fn resolve(self) -> SequenceEasing {
        match self {
            Self::Authored => SequenceEasing::Authored,
            Self::ReplacedBy(curve) => SequenceEasing::ReplacedBy(curve.resolve()),
            Self::ComposedWith(curve) => SequenceEasing::ComposedWith(curve.resolve()),
        }
    }
}

/// Spawns the thirds sequence with one producer already selected.
fn thirds_app(scope: ThirdsScope, easing: ThirdsEasing) -> (App, Entity, Entity, Entity) {
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 1);
    let member = members[0];
    let sequence = app.world_mut().spawn(thirds_sequence(member)).id();
    advance(&mut app, Duration::ZERO);
    let scope = scope.resolve(&app, sequence);
    let easing = easing.resolve();
    let driver = app
        .world_mut()
        .spawn((
            SequenceDriver::new(sequence),
            SequenceEvaluation::new(scope, easing),
            movement(HALFWAY),
        ))
        .id();
    advance(&mut app, Duration::ZERO);
    (app, sequence, member, driver)
}

/// Returns the identity of one authored stage of a live sequence.
fn stage_id(app: &App, sequence: Entity, ordinal: usize) -> SequenceStageId {
    let Some(authored) = app.world().get::<FoldSequence>(sequence) else {
        panic!("the authored sequence stays on its entity");
    };
    valid(
        "stage identity",
        authored.sequence_stages().stage_id(ordinal),
    )
}

#[test]
fn every_easing_mode_reaches_output_over_whole_sequence_single_stage_and_range_scopes() {
    // A multi-stage scope remaps position, so its expectations are the
    // sequence's: a replaced curve places position itself, and a composed one
    // hands the remapped position back to authored stage easing.
    for curve in [ThirdsCurve::StockCubic, ThirdsCurve::BoundedLookup] {
        for scope in [ThirdsScope::WholeSequence, ThirdsScope::FirstTwoStages] {
            let (start, end) = scope.extent();
            let progress = (HALFWAY - start) / (end - start);
            let remapped = curve.at(progress).mul_add(end - start, start);
            for (easing, expected) in [
                (ThirdsEasing::Authored, authored_thirds_fraction(HALFWAY)),
                (ThirdsEasing::ReplacedBy(curve), remapped),
                (
                    ThirdsEasing::ComposedWith(curve),
                    authored_thirds_fraction(remapped),
                ),
            ] {
                let (scoped, scoped_sequence, member, _) = thirds_app(scope, easing);
                assert_close(
                    eased_of(&scoped, scoped_sequence, member),
                    expected,
                    &format!("{easing:?} over {scope:?}"),
                );
                assert_close(
                    position_of(&scoped, scoped_sequence),
                    HALFWAY,
                    "raw local position is never rewritten by easing",
                );
            }
        }
    }

    // A one-stage scope eases the selected stage's own output instead of
    // remapping position, so its expectations are the stage's.
    for curve in [ThirdsCurve::StockCubic, ThirdsCurve::BoundedLookup] {
        let output = curve.at(HALFWAY);
        for (easing, expected) in [
            (
                ThirdsEasing::ReplacedBy(curve),
                output.mul_add(THIRD, THIRD),
            ),
            (
                ThirdsEasing::ComposedWith(curve),
                (output * output).mul_add(THIRD, THIRD),
            ),
        ] {
            let (staged, staged_sequence, member, _) = thirds_app(ThirdsScope::MiddleStage, easing);
            assert_close(
                eased_of(&staged, staged_sequence, member),
                expected,
                &format!("{easing:?} over one stage"),
            );
        }
    }
}

#[test]
fn stage_scope_output_easing_applies_only_inside_the_selected_stage() {
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 2);
    let selected_member = members[0];
    let other_member = members[1];
    let sequence = app
        .world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::new(SECOND, EaseFunction::Linear))
                .stage(FoldStage::from(selected_member))
                .stage(FoldStage::from(other_member))
                .build(),
        )
        .id();
    advance(&mut app, Duration::ZERO);
    let selected_stage = stage_id(&app, sequence, 0);
    app.world_mut().spawn((
        SequenceDriver::new(sequence),
        SequenceEvaluation::new(
            SequenceScope::Stage(selected_stage),
            SequenceEasing::ReplacedBy(Easing::from(EaseFunction::QuadraticIn)),
        ),
        movement(0.25),
    ));
    advance(&mut app, Duration::ZERO);

    // At a quarter of the sequence the first member is halfway through stage 0
    // and the second has not begun, so only the first is inside the selected
    // stage and only it takes the producer's curve.
    assert_close(
        eased_of(&app, sequence, selected_member),
        0.25,
        "the selected stage's member takes the producer's output curve",
    );
    assert_close(
        eased_of(&app, sequence, other_member),
        0.0,
        "a member outside the selected stage keeps its authored easing",
    );

    // Past the boundary the roles swap: the first rests at its reached target
    // under authored easing and the second travels under authored easing too.
    let driver = app_driver(&mut app, sequence);
    app.world_mut().entity_mut(driver).insert(movement(0.75));
    advance(&mut app, Duration::ZERO);
    assert_close(
        eased_of(&app, sequence, selected_member),
        1.0,
        "a member that left the selected stage rests at the target it reached",
    );
    assert_close(
        eased_of(&app, sequence, other_member),
        HALFWAY,
        "authored easing still runs outside the selected stage",
    );
}

/// Returns the producer currently driving `sequence`.
fn app_driver(app: &mut App, sequence: Entity) -> Entity {
    match owner_of(app, sequence) {
        SequenceOwner::Driver(driver) => driver,
        SequenceOwner::NativePlayback => panic!("no producer drives {sequence:?}"),
    }
}

#[test]
fn stage_scope_output_easing_may_overshoot_while_raw_traversal_stays_ordered() {
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 1);
    let member = members[0];
    let sequence = app
        .world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::new(SECOND, EaseFunction::Linear))
                .stage(FoldStage::from(member))
                .build(),
        )
        .id();
    advance(&mut app, Duration::ZERO);
    let only_stage = stage_id(&app, sequence, 0);
    let overshooting = overshooting_curve();
    let driver = app
        .world_mut()
        .spawn((
            SequenceDriver::new(sequence),
            SequenceEvaluation::new(
                SequenceScope::Stage(only_stage),
                SequenceEasing::ReplacedBy(Easing::from(overshooting)),
            ),
            movement(0.0),
        ))
        .id();

    let mut raw_positions = Vec::new();
    let mut fractions = Vec::new();
    for position in [0.0, 0.25, HALFWAY, 0.75, 1.0] {
        app.world_mut()
            .entity_mut(driver)
            .insert(movement(position));
        advance(&mut app, Duration::ZERO);
        raw_positions.push(position_of(&app, sequence));
        fractions.push(eased_of(&app, sequence, member));
    }

    assert_eq!(raw_positions, vec![0.0, 0.25, HALFWAY, 0.75, 1.0]);
    assert!(
        fractions[2] > 1.0,
        "a one-stage curve may carry output past the folded endpoint: {fractions:?}",
    );
    assert!(
        fractions[3] < fractions[2],
        "the same curve may reverse its output while raw travel stays ordered: {fractions:?}",
    );
}

#[test]
fn an_overshooting_curve_is_rejected_for_wide_scopes_and_warns_once_per_path() {
    for (scope_of, condition) in [
        (
            (|_, _| SequenceScope::WholeSequence) as fn(&App, Entity) -> SequenceScope,
            REJECTED_MULTI_STAGE_CURVE,
        ),
        (
            |app: &App, sequence: Entity| SequenceScope::StageRange {
                first: stage_id(app, sequence, 0),
                last:  stage_id(app, sequence, 1),
            },
            REJECTED_MULTI_STAGE_CURVE,
        ),
    ] {
        let warnings = WarnLog::default();
        let mut app = folding_app();
        let (_, members) = quarter_turn_members(&mut app, 1);
        let member = members[0];
        let sequence = app.world_mut().spawn(thirds_sequence(member)).id();
        advance(&mut app, Duration::ZERO);
        let overshooting = overshooting_curve();
        let bounded = bounded_curve();
        let scope = scope_of(&app, sequence);
        let driver = app
            .world_mut()
            .spawn((
                SequenceDriver::new(sequence),
                SequenceEvaluation::new(
                    scope,
                    SequenceEasing::ReplacedBy(Easing::from(overshooting)),
                ),
                movement(HALFWAY),
            ))
            .id();

        logged_advance(&warnings, &mut app, Duration::ZERO);
        let held = fraction_of(&app, sequence, member);
        logged_advance(&warnings, &mut app, Duration::ZERO);

        assert_eq!(
            warnings.naming(condition),
            1,
            "a rejected curve warns once per sequence entity",
        );
        assert_eq!(
            fraction_of(&app, sequence, member),
            held,
            "a rejected curve holds whatever the member already carries",
        );

        // Repairing the producer's curve rearms the slot, so a later rejection
        // is reported again.
        app.world_mut()
            .entity_mut(driver)
            .insert(SequenceEvaluation::new(
                scope,
                SequenceEasing::ReplacedBy(Easing::from(bounded)),
            ));
        logged_advance(&warnings, &mut app, Duration::ZERO);
        assert_eq!(
            warnings.naming(condition),
            1,
            "a clean sample warns nothing"
        );

        let rearmed = overshooting_curve();
        app.world_mut()
            .entity_mut(driver)
            .insert(SequenceEvaluation::new(
                scope,
                SequenceEasing::ReplacedBy(Easing::from(rearmed)),
            ));
        logged_advance(&warnings, &mut app, Duration::ZERO);
        assert_eq!(
            warnings.naming(condition),
            2,
            "a clean sample rearms the slot for the next rejection",
        );
    }
}

/// Marks a producer that reasserts its source state every update, which is what
/// makes it restorable after an explicit takeover releases the sequence.
#[derive(Component)]
struct RestorableProducer;

/// Reasserts every restorable producer's source state.
///
/// Shared arbitration clears source state at the end of
/// `SequencePlaybackSystems::ArbitrateDrivers` in `Update`, so marking in
/// `PostUpdate` is still current when the next update's release resolution
/// reads it.
fn mark_restorable_producers_current(
    mut producers: Query<&mut SequenceSourceState, With<RestorableProducer>>,
) {
    for mut source_state in &mut producers {
        source_state.mark_current();
    }
}

/// Spawns a producer that drives the whole sequence under authored easing.
fn spawn_producer(app: &mut App, sequence: Entity, position: f64) -> Entity {
    app.world_mut()
        .spawn((
            SequenceDriver::new(sequence),
            SequenceEvaluation::new(SequenceScope::WholeSequence, SequenceEasing::Authored),
            movement(position),
        ))
        .id()
}

#[test]
fn a_colocated_driver_with_movement_drives_the_sequence_and_one_without_it_holds() {
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 1);
    let member = members[0];
    let sequence = app.world_mut().spawn(thirds_sequence(member)).id();
    advance(&mut app, Duration::ZERO);

    let driver = spawn_producer(&mut app, sequence, HALFWAY);
    advance(&mut app, Duration::ZERO);
    assert_eq!(owner_of(&mut app, sequence), SequenceOwner::Driver(driver));
    assert_close(
        position_of(&app, sequence),
        HALFWAY,
        "the producer's movement drives position",
    );
    assert_close(
        eased_of(&app, sequence, member),
        authored_thirds_fraction(HALFWAY),
        "the driven position reaches fold output",
    );

    // The same producer without movement publishes nothing to apply, so local
    // position holds where the last applied movement left it even as time runs.
    app.world_mut()
        .entity_mut(driver)
        .remove::<SequenceMovement>();
    advance(&mut app, SECOND);
    assert_eq!(
        owner_of(&mut app, sequence),
        SequenceOwner::Driver(driver),
        "a producer without movement still owns the sequence",
    );
    assert_close(
        position_of(&app, sequence),
        HALFWAY,
        "a producer without movement moves nothing",
    );
}

#[test]
fn a_producer_scope_that_stops_resolving_is_rejected_and_the_pose_holds() {
    let warnings = WarnLog::default();
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 1);
    let member = members[0];
    let sequence = app.world_mut().spawn(thirds_sequence(member)).id();
    advance(&mut app, Duration::ZERO);

    let last_stage = stage_id(&app, sequence, 2);
    let driver = app
        .world_mut()
        .spawn((
            SequenceDriver::new(sequence),
            SequenceEvaluation::new(
                SequenceScope::Stage(last_stage),
                SequenceEasing::ReplacedBy(Easing::from(EaseFunction::QuadraticIn)),
            ),
            movement(HALFWAY),
            RestorableProducer,
        ))
        .id();
    logged_advance(&warnings, &mut app, Duration::ZERO);
    let held = pose_of(&app, member);
    assert_eq!(warnings.total(), 0, "a resolving scope reports nothing");

    // Replacing the authored sequence with a shorter one retires the stage the
    // producer selected, so its scope no longer resolves.
    app.world_mut().entity_mut(sequence).insert(
        FoldSequenceBuilder::new(FoldTiming::new(SECOND, EaseFunction::Linear))
            .stage(FoldStage::from(member))
            .build(),
    );
    logged_advance(&warnings, &mut app, Duration::ZERO);
    logged_advance(&warnings, &mut app, Duration::ZERO);

    assert_eq!(
        warnings.naming(STALE_PRODUCER_SCOPE),
        1,
        "a stale producer scope warns once per driver",
    );
    assert_eq!(
        pose_of(&app, member),
        held,
        "a stale scope holds the pose the member already carried",
    );
    assert_eq!(
        fraction_of(&app, sequence, member),
        FoldMemberFraction::Unresolved,
        "a stale scope resolves no new fraction and substitutes none",
    );

    // The same driver selecting an extent that resolves again rearms the slot
    // without rebuilding playback, which is what lets the next stale selection
    // report at all.
    select_scope(&mut app, driver, SequenceScope::WholeSequence);
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_eq!(
        warnings.naming(STALE_PRODUCER_SCOPE),
        1,
        "an update whose scope resolves reports nothing",
    );

    select_scope(&mut app, driver, SequenceScope::Stage(last_stage));
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_eq!(
        warnings.naming(STALE_PRODUCER_SCOPE),
        2,
        "the rearmed slot reports the same driver's next stale scope",
    );
}

/// Replaces which extent one producer selects, leaving its sequence untouched.
///
/// Reauthoring the sequence would rebuild retained playback and hand back a
/// freshly armed report slot, so a rearm assertion has to move the producer
/// instead.
fn select_scope(app: &mut App, driver: Entity, scope: SequenceScope) {
    app.world_mut()
        .entity_mut(driver)
        .insert(SequenceEvaluation::new(scope, SequenceEasing::Authored));
}

#[test]
fn an_ordinary_claim_is_rejected_while_an_explicit_takeover_displaces_the_selected_producer() {
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 1);
    let sequence = app.world_mut().spawn(thirds_sequence(members[0])).id();
    advance(&mut app, Duration::ZERO);

    let first = spawn_producer(&mut app, sequence, 0.25);
    advance(&mut app, Duration::ZERO);
    assert_eq!(owner_of(&mut app, sequence), SequenceOwner::Driver(first));

    let overlapping = spawn_producer(&mut app, sequence, 0.75);
    advance(&mut app, Duration::ZERO);
    assert_eq!(
        owner_of(&mut app, sequence),
        SequenceOwner::Driver(first),
        "an ordinary overlapping claim is rejected",
    );
    assert_close(
        position_of(&app, sequence),
        0.25,
        "a rejected claim applies no movement",
    );
    app.world_mut().entity_mut(overlapping).despawn();

    let takeover = spawn_producer(&mut app, sequence, 0.75);
    app.world_mut()
        .entity_mut(takeover)
        .insert(SequenceDriverTakeover);
    advance(&mut app, Duration::ZERO);
    assert_eq!(
        owner_of(&mut app, sequence),
        SequenceOwner::Driver(takeover),
        "an explicit takeover displaces the selected producer",
    );
    assert_close(
        position_of(&app, sequence),
        0.75,
        "the takeover's movement drives position",
    );
}

#[test]
fn a_released_takeover_restores_a_current_producer_and_leaves_a_stale_one_unowned() {
    for restorable in [true, false] {
        let mut app = folding_app();
        app.add_systems(PostUpdate, mark_restorable_producers_current);
        let (_, members) = quarter_turn_members(&mut app, 1);
        let member = members[0];
        let sequence = app.world_mut().spawn(thirds_sequence(member)).id();
        advance(&mut app, Duration::ZERO);

        let displaced = spawn_producer(&mut app, sequence, 0.25);
        if restorable {
            app.world_mut()
                .entity_mut(displaced)
                .insert(RestorableProducer);
        }
        advance(&mut app, Duration::ZERO);
        assert_eq!(
            owner_of(&mut app, sequence),
            SequenceOwner::Driver(displaced)
        );

        let takeover = spawn_producer(&mut app, sequence, 0.75);
        app.world_mut()
            .entity_mut(takeover)
            .insert(SequenceDriverTakeover);
        advance(&mut app, Duration::ZERO);
        assert_eq!(
            owner_of(&mut app, sequence),
            SequenceOwner::Driver(takeover)
        );

        // The displaced producer kept publishing movement all along, so a
        // restoration resumes at its current position rather than replaying the
        // position it held when it was displaced.
        app.world_mut().entity_mut(displaced).insert(movement(0.9));
        app.world_mut().entity_mut(takeover).despawn();
        advance(&mut app, Duration::ZERO);

        if restorable {
            assert_eq!(
                owner_of(&mut app, sequence),
                SequenceOwner::Driver(displaced),
                "a producer that marked its source current is restored",
            );
            assert_close(
                position_of(&app, sequence),
                0.9,
                "a restored producer resumes at its current time",
            );
            assert_close(
                eased_of(&app, sequence, member),
                authored_thirds_fraction(0.9),
                "restoration reaches fold output",
            );
        } else {
            assert_eq!(
                owner_of(&mut app, sequence),
                SequenceOwner::NativePlayback,
                "restoration to a producer that owns nothing current is rejected",
            );
            assert_close(
                position_of(&app, sequence),
                0.75,
                "an unowned sequence holds where the released producer left it",
            );
        }
    }
}

#[test]
fn a_movement_that_contradicts_local_position_warns_once_and_rearms() {
    let warnings = WarnLog::default();
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 1);
    let member = members[0];
    let sequence = app.world_mut().spawn(thirds_sequence(member)).id();
    advance(&mut app, Duration::ZERO);
    command(
        &mut app,
        sequence,
        SequenceOwner::NativePlayback,
        SequenceCommand::Play,
    );
    advance(&mut app, Duration::from_millis(1500));
    let held = position_of(&app, sequence);

    // A producer that claims backward travel while naming a position ahead of
    // local position contradicts itself, so nothing is applied.
    let driver = app
        .world_mut()
        .spawn((
            SequenceDriver::new(sequence),
            SequenceEvaluation::new(SequenceScope::WholeSequence, SequenceEasing::Authored),
            movement_toward(0.9, SequenceDirection::Backward),
        ))
        .id();
    logged_advance(&warnings, &mut app, Duration::ZERO);
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_eq!(
        warnings.naming(INVALID_MOVEMENT),
        1,
        "an invalid movement warns once"
    );
    assert_close(
        position_of(&app, sequence),
        held,
        "an invalid movement is never applied",
    );

    app.world_mut().entity_mut(driver).insert(movement(0.9));
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_close(position_of(&app, sequence), 0.9, "a valid movement applies");
    app.world_mut()
        .entity_mut(driver)
        .insert(movement_toward(1.0, SequenceDirection::Backward));
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_eq!(
        warnings.naming(INVALID_MOVEMENT),
        2,
        "an update that raised nothing rearms the slot",
    );
}

#[test]
fn a_step_moves_one_authored_boundary_position_not_one_stage() {
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 2);
    let (first, second) = (members[0], members[1]);
    // One stage, two members staggered inside it: the second only begins when
    // the first arrives, so the stage carries an interior boundary at its
    // midpoint.
    let sequence = app
        .world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::new(SECOND, EaseFunction::Linear))
                .stage(
                    FoldStage::from(valid(
                        "staggered group",
                        FoldGroup::try_new(first, [second]),
                    ))
                    .override_member_timings_with(|member_index, _| {
                        FoldTiming::new(SECOND, EaseFunction::Linear).with_start_offset(
                            SECOND.saturating_mul(u32::try_from(member_index).unwrap_or_default()),
                        )
                    }),
                )
                .build(),
        )
        .id();
    advance(&mut app, Duration::ZERO);

    // A step names the next authored boundary as a destination; travel reaches
    // it over time and stops there, so each advance is long enough to arrive.
    let mut positions = Vec::new();
    for _ in 0..4 {
        command(
            &mut app,
            sequence,
            SequenceOwner::NativePlayback,
            SequenceCommand::Step,
        );
        advance(&mut app, Duration::from_secs(10));
        positions.push(position_of(&app, sequence));
    }

    assert!(
        positions.contains(&HALFWAY),
        "a step stops at the member boundary inside the stage: {positions:?}",
    );
    assert_close(
        positions[positions.len() - 1],
        1.0,
        "stepping past the last boundary rests at the sequence end",
    );
    assert!(
        positions.windows(2).all(|pair| pair[0] <= pair[1]),
        "stepping never travels backward: {positions:?}",
    );
    assert_close(
        eased_of(&app, sequence, first),
        1.0,
        "the first member arrived before the second began",
    );
    assert_close(
        eased_of(&app, sequence, second),
        1.0,
        "the second member arrived last",
    );
}

#[test]
fn member_fraction_names_untracked_and_eased_states() {
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 2);
    let (staged, unstaged) = (members[0], members[1]);
    let sequence = app
        .world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::new(Duration::from_secs(2), bounded_curve()))
                .stage(FoldStage::from(staged))
                .build(),
        )
        .id();
    advance(&mut app, Duration::ZERO);

    assert_eq!(
        fraction_of(&app, sequence, unstaged),
        FoldMemberFraction::Untracked,
        "a member no retained sequence stages is untracked",
    );
    assert!(
        matches!(
            fraction_of(&app, sequence, staged),
            FoldMemberFraction::Eased(_)
        ),
        "an owned curve resolves an eased fraction",
    );
}

#[test]
fn every_fold_evaluation_error_names_the_condition_that_produced_it() {
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 1);
    let member = members[0];
    let sequence = FoldSequenceBuilder::new(FoldTiming::new(SECOND, EaseFunction::Linear))
        .stage(FoldStage::from(member))
        .build();
    let sample = sequence.sample_member(member, HALFWAY);
    let authored_applies = |progress: f32| SequenceEasingSample::AuthoredEasingApplies { progress };
    let eased = |_: &Easing, progress: f32| EasingSample::Eased(progress);

    assert_eq!(
        fold_fraction(
            &sample,
            |_| SequenceEasingSample::CurveRejected(
                SequenceEasingError::MappingNotBoundedMonotonic
            ),
            eased,
        ),
        Err(FoldEvaluationError::ExternalCurveRejected(
            SequenceEasingError::MappingNotBoundedMonotonic
        )),
        "a rejected producer curve carries the shared error through",
    );
    assert_eq!(
        fold_fraction(&sample, authored_applies, |_, _| EasingSample::NonFinite),
        Err(FoldEvaluationError::NonFiniteEasing),
        "non-finite authored easing names the non-finite output",
    );

    // A hinge whose folded endpoint is a large angle carries an overshooting
    // stage-scope output past the finite range the interpolated angle needs.
    let unrepresentable = folded_hinge(0.0, f32::MAX, Displacement::default());
    assert_eq!(
        evaluate_fold_angle(&unrepresentable, &sample, authored_applies, |_, _| {
            EasingSample::Eased(f32::MAX)
        }),
        Err(FoldEvaluationError::UnrepresentableAngle),
        "an interpolated angle outside the finite range names itself",
    );
}

#[test]
fn a_stage_output_curve_rejection_and_a_non_finite_easing_each_warn_once_and_rearm() {
    assert_stage_output_curve_rejection_rearms();
    assert_non_finite_easing_rearms();
}

/// Selects one stage's output easing from a lookup curve.
fn select_stage_curve(app: &mut App, driver: Entity, stage: SequenceStageId, curve: EasingCurve) {
    app.world_mut()
        .entity_mut(driver)
        .insert(SequenceEvaluation::new(
            SequenceScope::Stage(stage),
            SequenceEasing::ReplacedBy(Easing::from(curve)),
        ));
}

/// A producer curve that cannot ease the stage it selected warns once per
/// driver, and an update that accepted a curve rearms the slot.
fn assert_stage_output_curve_rejection_rearms() {
    let warnings = WarnLog::default();
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 1);
    let member = members[0];
    let sequence = app.world_mut().spawn(thirds_sequence(member)).id();
    advance(&mut app, Duration::ZERO);
    let only_stage = stage_id(&app, sequence, 1);
    let overflowing = overflowing_curve();
    let bounded = bounded_curve();

    let driver = app
        .world_mut()
        .spawn((
            SequenceDriver::new(sequence),
            SequenceEvaluation::new(
                SequenceScope::Stage(only_stage),
                SequenceEasing::ReplacedBy(Easing::from(overflowing.clone())),
            ),
            movement(HALFWAY),
        ))
        .id();
    logged_advance(&warnings, &mut app, Duration::ZERO);
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_eq!(
        warnings.naming(REJECTED_STAGE_OUTPUT_CURVE),
        1,
        "a curve rejected as stage output easing warns once per driver",
    );

    select_stage_curve(&mut app, driver, only_stage, bounded);
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_eq!(
        warnings.naming(REJECTED_STAGE_OUTPUT_CURVE),
        1,
        "an update whose curve was accepted reports nothing",
    );

    select_stage_curve(&mut app, driver, only_stage, overflowing);
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_eq!(
        warnings.naming(REJECTED_STAGE_OUTPUT_CURVE),
        2,
        "the rearmed slot reports the same driver's next rejection",
    );
}

/// A sequence whose own authored easing names no output warns once per
/// sequence, and an update in which no member overflowed rearms the slot.
///
/// The condition needs the overflowing curve as the sequence's authored
/// `FoldTiming` easing: a producer's external curve is rejected before it is
/// ever sampled, which is a different slot.
fn assert_non_finite_easing_rearms() {
    let warnings = WarnLog::default();
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 1);
    let member = members[0];
    let overflowing = overflowing_curve();
    let sequence = app
        .world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::new(Duration::from_secs(4), overflowing))
                .stage(FoldStage::from(member))
                .build(),
        )
        .id();
    advance(&mut app, Duration::ZERO);

    // The authored curve is finite at its knots and overflows between them, so
    // the sequence interior is where it names no output at all.
    let driver = spawn_producer(&mut app, sequence, HALFWAY);
    logged_advance(&warnings, &mut app, Duration::ZERO);
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_eq!(
        warnings.naming(NON_FINITE_EASING),
        1,
        "a non-finite authored easing warns once per sequence",
    );

    // Travelling back to the start has to name the backward direction, or the
    // movement contradicts local position and is not applied at all.
    app.world_mut()
        .entity_mut(driver)
        .insert(movement_toward(0.0, SequenceDirection::Backward));
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_close(
        position_of(&app, sequence),
        0.0,
        "the producer carried the sequence back to its start",
    );
    assert_eq!(
        warnings.naming(NON_FINITE_EASING),
        1,
        "the sequence start eases finitely and reports nothing",
    );

    app.world_mut().entity_mut(driver).insert(movement(HALFWAY));
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_eq!(
        warnings.naming(NON_FINITE_EASING),
        2,
        "the rearmed slot reports the next non-finite output",
    );
}

#[test]
fn an_interpolated_angle_that_leaves_the_finite_range_holds_the_pose_it_reached() {
    let warnings = WarnLog::default();
    let mut app = folding_app();
    let target = app.world_mut().spawn_empty().id();
    // The folded endpoint is representable on its own; carrying the fraction
    // past it is what leaves the range.
    let member = app
        .world_mut()
        .spawn((
            hinge_geometry(),
            AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
            folded_hinge(QUARTER_TURN, f32::MAX, Displacement::default()),
        ))
        .id();
    let sequence = app
        .world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::new(SECOND, EaseFunction::Linear))
                .stage(FoldStage::from(member))
                .build(),
        )
        .id();
    logged_advance(&warnings, &mut app, Duration::ZERO);
    let held = pose_of(&app, member);
    assert_ne!(
        held,
        AnchorPose::default(),
        "the base angle poses the member before anything overflows",
    );

    let only_stage = stage_id(&app, sequence, 0);
    let overshooting = overshooting_curve();
    app.world_mut().spawn((
        SequenceDriver::new(sequence),
        SequenceEvaluation::new(
            SequenceScope::Stage(only_stage),
            SequenceEasing::ReplacedBy(Easing::from(overshooting)),
        ),
        movement(HALFWAY),
    ));
    logged_advance(&warnings, &mut app, Duration::ZERO);

    assert!(
        eased_of(&app, sequence, member) > 1.0,
        "the stage output curve carried the fraction past the folded endpoint",
    );
    assert_eq!(
        pose_of(&app, member),
        held,
        "an unrepresentable angle holds the pose the hinge already carried",
    );
    assert!(
        app.world().entity(member).contains::<HingePoseReported>(),
        "the hinge that could not pose carries the report",
    );
    assert_eq!(
        warnings.naming(HINGE_POSE_UNAVAILABLE),
        1,
        "an unrepresentable angle warns once while the condition lasts",
    );
}

#[test]
fn a_command_against_an_entity_with_no_retained_playback_is_not_a_rejection() {
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 1);
    let sequence = app.world_mut().spawn(thirds_sequence(members[0])).id();
    let bare = app.world_mut().spawn_empty().id();
    advance(&mut app, Duration::ZERO);

    assert_eq!(
        command(
            &mut app,
            bare,
            SequenceOwner::NativePlayback,
            SequenceCommand::Play
        ),
        SequenceCommandResponse::NoRetainedSequence,
        "an entity carrying no retained playback answers that it holds none",
    );

    let driver = spawn_producer(&mut app, sequence, HALFWAY);
    advance(&mut app, Duration::ZERO);
    assert_eq!(
        command(
            &mut app,
            sequence,
            SequenceOwner::NativePlayback,
            SequenceCommand::Play
        ),
        SequenceCommandResponse::Rejected(SequenceOwner::Driver(driver)),
        "a sequence an owner holds names that owner instead",
    );
}

/// Gives one arrangement member the hinge, geometry, and attachment a pose
/// needs, none of which a bare sheet plan supplies.
fn pose_member(app: &mut App, member: Entity, target: Entity) {
    app.world_mut().entity_mut(member).insert((
        hinge_geometry(),
        AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
        folded_hinge(0.0, QUARTER_TURN, Displacement::default()),
    ));
}

#[test]
fn a_group_the_closure_omitted_rests_at_its_base_endpoint_while_its_siblings_fold() {
    let mut app = folding_app();
    // The closure keeps the first group the provider offered and drops the
    // rest, so every later group's members stay out of the authored sequence.
    let arrangement = spawn_sheet(
        &mut app,
        QuadSheet::new(2, 2).with_custom_fold_sequence(
            QuadFoldGroupSelection::Rows,
            |groups: &FoldGroups| {
                Ok(
                    FoldSequenceBuilder::new(FoldTiming::new(SECOND, EaseFunction::Linear))
                        .stages(groups.iter().take(1).cloned())
                        .build(),
                )
            },
        ),
    );
    let groups = retained_groups(&app, arrangement, &QuadFoldGroupSelection::Rows);
    let [staged_group, omitted_group, ..] = groups.as_slice() else {
        panic!("the row alternative offers more than one group: {groups:?}");
    };
    let (Some(&folding), Some(&omitted)) = (staged_group.first(), omitted_group.first()) else {
        panic!("every retained group names at least one member: {groups:?}");
    };
    let target = app.world_mut().spawn_empty().id();
    pose_member(&mut app, folding, target);
    pose_member(&mut app, omitted, target);

    advance(&mut app, Duration::ZERO);
    command(
        &mut app,
        arrangement,
        SequenceOwner::NativePlayback,
        SequenceCommand::Play,
    );
    advance(&mut app, SECOND);

    let Some(authored) = app.world().get::<FoldSequence>(arrangement) else {
        panic!("the custom adapter authored a sequence on the controller");
    };
    assert_eq!(
        authored.sample_member(omitted, 1.0),
        FoldMemberSample::Unauthored,
        "a member no stage names is unauthored at every position",
    );
    assert_close(
        valid(
            "unauthored fraction",
            fold_fraction(
                &FoldMemberSample::Unauthored,
                |progress| SequenceEasingSample::AuthoredEasingApplies { progress },
                |_, progress| EasingSample::Eased(progress),
            ),
        )
        .value(),
        f64::from(FoldTarget::BASE.fraction()),
        "an unauthored member holds its base endpoint",
    );
    // Untracked is what separates an omitted member from a staged one that has
    // resolved nothing yet; both would carry the default pose.
    assert_eq!(
        fraction_of(&app, arrangement, omitted),
        FoldMemberFraction::Untracked,
        "a member the closure omitted is tracked by no retained sequence",
    );
    assert_close(
        eased_of(&app, arrangement, folding),
        1.0,
        "its sibling folded fully",
    );
    assert_ne!(
        pose_of(&app, folding),
        AnchorPose::default(),
        "the staged sibling reached a folded pose",
    );
    assert_eq!(
        pose_of(&app, omitted),
        AnchorPose::default(),
        "an unauthored member is never posed away from its base",
    );
}

#[test]
fn a_sequence_naming_hinge_less_entities_evaluates_the_rest_and_warns_once() {
    let warnings = WarnLog::default();
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 1);
    let hinged = members[0];
    let hinge_less = app.world_mut().spawn_empty().id();
    let sequence = app
        .world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::new(SECOND, EaseFunction::Linear))
                .stage(FoldStage::from(valid(
                    "partly hinged group",
                    FoldGroup::try_new(hinged, [hinge_less]),
                )))
                .build(),
        )
        .id();
    logged_advance(&warnings, &mut app, Duration::ZERO);
    command(
        &mut app,
        sequence,
        SequenceOwner::NativePlayback,
        SequenceCommand::Play,
    );
    logged_advance(&warnings, &mut app, SECOND);
    logged_advance(&warnings, &mut app, Duration::ZERO);

    assert_close(
        eased_of(&app, sequence, hinged),
        1.0,
        "the hinged member evaluates while its hinge-less partner is skipped",
    );
    assert_eq!(
        warnings.naming(HINGE_LESS_TRACKS),
        1,
        "hinge-less tracks warn once per sequence entity",
    );

    // An update in which every staged member carries a hinge rearms the slot,
    // so the same gap reappearing reports again.
    app.world_mut().entity_mut(hinge_less).insert(folded_hinge(
        0.0,
        QUARTER_TURN,
        Displacement::default(),
    ));
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_eq!(
        warnings.naming(HINGE_LESS_TRACKS),
        1,
        "an update that skipped no member reports nothing",
    );

    app.world_mut().entity_mut(hinge_less).remove::<Hinge>();
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_eq!(
        warnings.naming(HINGE_LESS_TRACKS),
        2,
        "the rearmed slot reports the gap the next time it appears",
    );
}

/// Counts the updates in which at least one hinge's pose actually changed.
#[derive(Default, Resource)]
struct PoseWrites(usize);

/// Records one update in which a hinge pose changed.
fn record_pose_writes(changed: Query<(), Changed<AnchorPose>>, mut writes: ResMut<PoseWrites>) {
    if !changed.is_empty() {
        writes.0 += 1;
    }
}

/// Returns how many pose writes happened since the last call and resets the tally.
fn take_pose_writes(app: &mut App) -> usize {
    let mut writes = app.world_mut().resource_mut::<PoseWrites>();
    let total = writes.0;
    writes.0 = 0;
    total
}

#[test]
fn a_hinge_that_cannot_be_posed_reports_once_and_clears_the_report_when_it_can() {
    let warnings = WarnLog::default();
    let mut app = folding_app();
    let (target, members) = quarter_turn_members(&mut app, 1);
    let member = members[0];
    let sequence = app
        .world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::new(SECOND, EaseFunction::Linear))
                .stage(FoldStage::from(member))
                .build(),
        )
        .id();
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert!(
        !app.world().entity(member).contains::<HingePoseReported>(),
        "a hinge that poses cleanly carries no report",
    );

    // Without the attachment the hinge has no target frame to pose against.
    app.world_mut().entity_mut(member).remove::<AnchoredTo>();
    logged_advance(&warnings, &mut app, Duration::ZERO);
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert!(
        app.world().entity(member).contains::<HingePoseReported>(),
        "a hinge holding a stale pose carries the report",
    );
    assert_eq!(
        warnings.naming(HINGE_POSE_UNAVAILABLE),
        1,
        "an unavailable hinge pose warns once while the condition lasts",
    );

    app.world_mut().entity_mut(member).insert(AnchoredTo::new(
        target,
        AnchorSite::Center,
        AnchorSite::Center,
    ));
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert!(
        !app.world().entity(member).contains::<HingePoseReported>(),
        "a successful pose removes the report",
    );

    app.world_mut().entity_mut(member).remove::<AnchoredTo>();
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_eq!(
        warnings.naming(HINGE_POSE_UNAVAILABLE),
        2,
        "removing the report rearms the warning for the next occurrence",
    );
    assert!(
        matches!(
            fraction_of(&app, sequence, member),
            FoldMemberFraction::Eased(_)
        ),
        "the sequence keeps resolving the member's fraction while its pose is unavailable",
    );
}

#[test]
fn a_member_that_keeps_raising_a_condition_never_lets_a_clean_sibling_rearm_the_slot() {
    let warnings = WarnLog::default();
    let mut app = folding_app();
    let (_, members) = quarter_turn_members(&mut app, 2);
    let (clean, broken) = (members[0], members[1]);
    let overflowing = overflowing_curve();
    let sequence = app
        .world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::new(
                Duration::from_secs(4),
                EaseFunction::Linear,
            ))
            .stage(valid(
                "mixed stage",
                FoldStage::from(valid("mixed group", FoldGroup::try_new(clean, [broken])))
                    .override_member_timing(
                        broken,
                        FoldTiming::new(Duration::from_secs(4), overflowing),
                    ),
            ))
            .build(),
        )
        .id();
    advance(&mut app, Duration::ZERO);
    command(
        &mut app,
        sequence,
        SequenceOwner::NativePlayback,
        SequenceCommand::Play,
    );

    // The overflowing curve is finite at its authored knots and non-finite
    // between them, so this travels into the middle of the sequence, where it
    // overflows, and stays there.
    for _ in 0..5 {
        logged_advance(&warnings, &mut app, Duration::from_millis(500));
    }
    let held = fraction_of(&app, sequence, broken);
    assert!(
        matches!(held, FoldMemberFraction::Eased(_)),
        "the member holds the last fraction it resolved before overflowing",
    );
    assert_eq!(
        warnings.naming(NON_FINITE_EASING),
        1,
        "the overflow is reported once",
    );

    // Every further update raises it again while the clean sibling resolves
    // normally, and the clean sibling never rearms the slot.
    for _ in 0..2 {
        logged_advance(&warnings, &mut app, Duration::ZERO);
        assert!(
            matches!(
                fraction_of(&app, sequence, clean),
                FoldMemberFraction::Eased(_)
            ),
            "the clean sibling resolves every update",
        );
    }
    assert_eq!(
        fraction_of(&app, sequence, broken),
        held,
        "the sibling whose easing overflows holds whatever it already carried",
    );
    assert_eq!(
        warnings.naming(NON_FINITE_EASING),
        1,
        "a clean sibling never rearms a slot the broken one still raises",
    );
}

/// Runs one update and returns whether it wrote a pose, clearing the tally.
fn writes_after(warnings: &WarnLog, app: &mut App, delta: Duration) -> usize {
    logged_advance(warnings, app, delta);
    take_pose_writes(app)
}

/// Captured input starts traversal, time carries it, and a pause returns the
/// sequence to idle.
fn assert_input_and_traversal_write(warnings: &WarnLog, app: &mut App, sequence: Entity) {
    command(
        app,
        sequence,
        SequenceOwner::NativePlayback,
        SequenceCommand::Play,
    );
    assert_eq!(
        writes_after(warnings, app, SECOND),
        1,
        "captured input writes one pose"
    );
    assert_eq!(
        writes_after(warnings, app, SECOND),
        1,
        "traversal writes one pose"
    );

    command(
        app,
        sequence,
        SequenceOwner::NativePlayback,
        SequenceCommand::Pause,
    );
    let _ = writes_after(warnings, app, Duration::ZERO);
    assert_eq!(
        writes_after(warnings, app, Duration::ZERO),
        0,
        "a paused fold is idle again"
    );
}

/// A producer takes the sequence to a different position, and reauthoring it
/// lands the same position somewhere else.
fn assert_ownership_and_authoring_write(
    warnings: &WarnLog,
    app: &mut App,
    sequence: Entity,
    member: Entity,
    authored: &EasingCurve,
) {
    let driver = spawn_producer(app, sequence, 0.75);
    assert_eq!(
        writes_after(warnings, app, Duration::ZERO),
        1,
        "a change of owner writes one pose"
    );

    app.world_mut().entity_mut(driver).despawn();
    let _ = writes_after(warnings, app, Duration::ZERO);
    app.world_mut().entity_mut(sequence).insert(
        FoldSequenceBuilder::new(FoldTiming::new(Duration::from_secs(4), authored.clone()))
            .stage(valid(
                "retimed stage",
                FoldStage::from(member).with_member_target(member, FoldTarget::BASE),
            ))
            .build(),
    );
    assert_eq!(
        writes_after(warnings, app, Duration::ZERO),
        1,
        "new authoring writes one pose"
    );
}

#[test]
fn an_idle_fold_writes_no_pose_while_every_change_trigger_writes_one() {
    let warnings = WarnLog::default();
    let mut app = folding_app();
    app.init_resource::<PoseWrites>().add_systems(
        PostUpdate,
        record_pose_writes.after(AnchorSystems::AnimatePose),
    );
    let (_, members) = quarter_turn_members(&mut app, 1);
    let member = members[0];
    let authored = bounded_curve();
    let sequence = app
        .world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::new(Duration::from_secs(4), authored.clone()))
                .stage(FoldStage::from(member))
                .build(),
        )
        .id();
    let _ = writes_after(&warnings, &mut app, Duration::ZERO);

    // Equal progress: nothing moved, so nothing is written.
    assert_eq!(
        writes_after(&warnings, &mut app, Duration::ZERO),
        0,
        "an idle fold writes no pose"
    );

    assert_input_and_traversal_write(&warnings, &mut app, sequence);
    assert_ownership_and_authoring_write(&warnings, &mut app, sequence, member, &authored);

    app.world_mut().entity_mut(sequence).insert(
        FoldSequenceBuilder::new(FoldTiming::new(Duration::from_secs(4), authored.clone()))
            .stage(FoldStage::from(member))
            .build(),
    );
    let _ = writes_after(&warnings, &mut app, Duration::ZERO);

    command(
        &mut app,
        sequence,
        SequenceOwner::NativePlayback,
        SequenceCommand::Play,
    );
    let _ = writes_after(&warnings, &mut app, SECOND);

    // A replaced hinge changes the endpoints the same fraction interpolates.
    app.world_mut().entity_mut(member).insert(folded_hinge(
        0.0,
        -QUARTER_TURN,
        Displacement::default(),
    ));
    assert_eq!(
        writes_after(&warnings, &mut app, Duration::ZERO),
        1,
        "a replaced hinge writes one pose"
    );

    assert_eq!(warnings.total(), 0, "none of these transitions is an error");
}

// Fold boundary events: order, exact raw timing, targets, and traversal.

const TWO_SECONDS: Duration = Duration::from_secs(2);

/// What one recorded event named, flattened out of the five event types.
#[derive(Clone, Copy, Debug, PartialEq)]
enum FoldEventSubject {
    StageBegan(usize),
    StageEnded(usize),
    MemberBegan { member: Entity, stage: usize },
    MemberEnded { member: Entity, stage: usize },
    EndpointReached(FoldEndpoint),
}

/// One emitted boundary event, with the entity it was triggered on.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RecordedFoldEvent {
    subject:     FoldEventSubject,
    target:      Entity,
    arrangement: Entity,
    direction:   SequenceDirection,
    timing:      FoldEventTiming,
}

/// Every boundary event this app emitted, in trigger order.
#[derive(Resource, Default)]
struct FoldEventLog(Vec<RecordedFoldEvent>);

impl FoldEventLog {
    fn record(
        &mut self,
        subject: FoldEventSubject,
        target: Entity,
        arrangement: Entity,
        direction: SequenceDirection,
        timing: FoldEventTiming,
    ) {
        self.0.push(RecordedFoldEvent {
            subject,
            target,
            arrangement,
            direction,
            timing,
        });
    }
}

fn record_stage_begin(began: On<FoldStageBegin>, mut log: ResMut<FoldEventLog>) {
    log.record(
        FoldEventSubject::StageBegan(began.stage.ordinal()),
        began.event_target(),
        began.arrangement,
        began.direction,
        began.timing,
    );
}

fn record_stage_end(ended: On<FoldStageEnd>, mut log: ResMut<FoldEventLog>) {
    log.record(
        FoldEventSubject::StageEnded(ended.stage.ordinal()),
        ended.event_target(),
        ended.arrangement,
        ended.direction,
        ended.timing,
    );
}

fn record_member_begin(began: On<FoldMemberBegin>, mut log: ResMut<FoldEventLog>) {
    log.record(
        FoldEventSubject::MemberBegan {
            member: began.member,
            stage:  began.stage.ordinal(),
        },
        began.event_target(),
        began.arrangement,
        began.direction,
        began.timing,
    );
}

fn record_member_end(ended: On<FoldMemberEnd>, mut log: ResMut<FoldEventLog>) {
    log.record(
        FoldEventSubject::MemberEnded {
            member: ended.member,
            stage:  ended.stage.ordinal(),
        },
        ended.event_target(),
        ended.arrangement,
        ended.direction,
        ended.timing,
    );
}

fn record_endpoint_reached(reached: On<FoldEndpointReached>, mut log: ResMut<FoldEventLog>) {
    log.record(
        FoldEventSubject::EndpointReached(reached.endpoint),
        reached.event_target(),
        reached.arrangement,
        reached.direction,
        reached.timing,
    );
}

/// A folding app that records every boundary event its sequences emit.
fn recording_fold_app() -> App {
    let mut app = folding_app();
    app.init_resource::<FoldEventLog>()
        .add_observer(record_stage_begin)
        .add_observer(record_stage_end)
        .add_observer(record_member_begin)
        .add_observer(record_member_end)
        .add_observer(record_endpoint_reached);
    app
}

/// Takes every event recorded since the last drain.
fn drained_events(app: &mut App) -> Vec<RecordedFoldEvent> {
    let mut log = app.world_mut().resource_mut::<FoldEventLog>();
    core::mem::take(&mut log.0)
}

fn subjects_of(events: &[RecordedFoldEvent]) -> Vec<FoldEventSubject> {
    events.iter().map(|event| event.subject).collect()
}

fn drained_subjects(app: &mut App) -> Vec<FoldEventSubject> { subjects_of(&drained_events(app)) }

/// One boundary event named without any entity a world allocated.
///
/// Two independently built `App`s hand out their own entity ids, so a member is
/// named by its index in that app's own staged member list instead of by the
/// `Entity` the event carried.
#[derive(Clone, Copy, Debug, PartialEq)]
enum CrossAppFoldBoundary {
    StageBegan(usize),
    StageEnded(usize),
    MemberBegan {
        member_index: usize,
        stage:        usize,
    },
    MemberEnded {
        member_index: usize,
        stage:        usize,
    },
    EndpointReached(FoldEndpoint),
}

/// Names every recorded boundary in the form two apps can compare.
fn cross_app_boundaries(
    events: &[RecordedFoldEvent],
    members: &[Entity],
) -> Vec<CrossAppFoldBoundary> {
    let member_index = |member: Entity| {
        members
            .iter()
            .position(|staged| *staged == member)
            .expect("every member event names one of the app's own staged members")
    };
    events
        .iter()
        .map(|event| match event.subject {
            FoldEventSubject::StageBegan(stage) => CrossAppFoldBoundary::StageBegan(stage),
            FoldEventSubject::StageEnded(stage) => CrossAppFoldBoundary::StageEnded(stage),
            FoldEventSubject::MemberBegan { member, stage } => CrossAppFoldBoundary::MemberBegan {
                member_index: member_index(member),
                stage,
            },
            FoldEventSubject::MemberEnded { member, stage } => CrossAppFoldBoundary::MemberEnded {
                member_index: member_index(member),
                stage,
            },
            FoldEventSubject::EndpointReached(endpoint) => {
                CrossAppFoldBoundary::EndpointReached(endpoint)
            },
        })
        .collect()
}

/// Spawns two members in two consecutive one-second stages.
///
/// Its ledger is the documented coincident order: the first stage's begin, its
/// member's begin, and the base endpoint at the start; the first member's end,
/// the first stage's end, the second stage's begin, and its member's begin at
/// the halfway mark; the second member's end, the second stage's end, and the
/// folded endpoint at the end.
fn two_stage_sequence(app: &mut App) -> (Entity, [Entity; 2]) {
    let (_, members) = quarter_turn_members(app, 2);
    let sequence = app
        .world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::new(SECOND, EaseFunction::Linear))
                .stage(FoldStage::from(members[0]))
                .stage(FoldStage::from(members[1]))
                .build(),
        )
        .id();
    advance(app, Duration::ZERO);
    (sequence, [members[0], members[1]])
}

/// The forward ledger of [`two_stage_sequence`], in emission order.
fn forward_two_stage_subjects(members: [Entity; 2]) -> Vec<FoldEventSubject> {
    vec![
        FoldEventSubject::StageBegan(0),
        FoldEventSubject::MemberBegan {
            member: members[0],
            stage:  0,
        },
        FoldEventSubject::EndpointReached(FoldEndpoint::Base),
        FoldEventSubject::MemberEnded {
            member: members[0],
            stage:  0,
        },
        FoldEventSubject::StageEnded(0),
        FoldEventSubject::StageBegan(1),
        FoldEventSubject::MemberBegan {
            member: members[1],
            stage:  1,
        },
        FoldEventSubject::MemberEnded {
            member: members[1],
            stage:  1,
        },
        FoldEventSubject::StageEnded(1),
        FoldEventSubject::EndpointReached(FoldEndpoint::Folded),
    ]
}

/// The entity each subject is triggered on: a member event its member, every
/// other event the arrangement.
const fn expected_target(subject: FoldEventSubject, arrangement: Entity) -> Entity {
    match subject {
        FoldEventSubject::MemberBegan { member, .. }
        | FoldEventSubject::MemberEnded { member, .. } => member,
        FoldEventSubject::StageBegan(_)
        | FoldEventSubject::StageEnded(_)
        | FoldEventSubject::EndpointReached(_) => arrangement,
    }
}

/// Plays one retained sequence forward to its folded endpoint in one update.
fn play_forward(app: &mut App, sequence: Entity, delta: Duration) {
    command(
        app,
        sequence,
        SequenceOwner::NativePlayback,
        SequenceCommand::Play,
    );
    advance(app, delta);
}

#[test]
fn every_crossed_boundary_emits_once_in_ledger_order_with_its_exact_raw_timing() {
    let mut app = recording_fold_app();
    let (sequence, members) = two_stage_sequence(&mut app);
    assert!(
        drained_events(&mut app).is_empty(),
        "building retained playback crosses no boundary",
    );

    play_forward(&mut app, sequence, TWO_SECONDS);

    let emitted = drained_events(&mut app);
    assert_eq!(subjects_of(&emitted), forward_two_stage_subjects(members));

    // Exact raw ledger time, whole-sequence extent, and normalized place of
    // each crossed record, in the same order.
    let expected_places = [
        (Duration::ZERO, 0.0),
        (Duration::ZERO, 0.0),
        (Duration::ZERO, 0.0),
        (SECOND, HALFWAY),
        (SECOND, HALFWAY),
        (SECOND, HALFWAY),
        (SECOND, HALFWAY),
        (TWO_SECONDS, 1.0),
        (TWO_SECONDS, 1.0),
        (TWO_SECONDS, 1.0),
    ];
    assert_eq!(emitted.len(), expected_places.len());
    for (event, (elapsed, position)) in emitted.iter().zip(expected_places) {
        assert_eq!(
            event.timing.elapsed(),
            SequenceTime::from(elapsed),
            "{:?} carries its own authored time",
            event.subject,
        );
        assert_eq!(event.timing.total(), SequenceTime::from(TWO_SECONDS));
        assert_close(
            f64::from(event.timing.position().normalized()),
            position,
            "boundary position",
        );
        assert_eq!(event.direction, SequenceDirection::Forward);
        assert_eq!(event.arrangement, sequence);
        assert_eq!(
            event.target,
            expected_target(event.subject, sequence),
            "{:?} targets its own entity",
            event.subject,
        );
    }
}

#[test]
fn backward_travel_emits_the_reversed_ledger_with_begin_and_end_swapped() {
    let mut app = recording_fold_app();
    let (sequence, members) = two_stage_sequence(&mut app);
    play_forward(&mut app, sequence, TWO_SECONDS);
    let _ = drained_events(&mut app);

    command(
        &mut app,
        sequence,
        SequenceOwner::NativePlayback,
        SequenceCommand::PlayBackward,
    );
    advance(&mut app, TWO_SECONDS);

    let emitted = drained_events(&mut app);
    // Crossing a stage's authored end while travelling backward starts that
    // stage, so the reversed ledger reads as a backward run rather than as a
    // forward one played in reverse.
    assert_eq!(
        subjects_of(&emitted),
        [
            FoldEventSubject::EndpointReached(FoldEndpoint::Folded),
            FoldEventSubject::StageBegan(1),
            FoldEventSubject::MemberBegan {
                member: members[1],
                stage:  1,
            },
            FoldEventSubject::MemberEnded {
                member: members[1],
                stage:  1,
            },
            FoldEventSubject::StageEnded(1),
            FoldEventSubject::StageBegan(0),
            FoldEventSubject::MemberBegan {
                member: members[0],
                stage:  0,
            },
            FoldEventSubject::EndpointReached(FoldEndpoint::Base),
            FoldEventSubject::MemberEnded {
                member: members[0],
                stage:  0,
            },
            FoldEventSubject::StageEnded(0),
        ]
    );
    assert!(
        emitted
            .iter()
            .all(|event| event.direction == SequenceDirection::Backward),
        "every backward crossing carries the direction it was crossed in",
    );
}

#[test]
fn arbitrary_seeks_emit_only_the_boundaries_between_their_two_positions() {
    let mut app = recording_fold_app();
    let (sequence, members) = two_stage_sequence(&mut app);
    let driver = app
        .world_mut()
        .spawn((SequenceDriver::new(sequence), movement(0.75)))
        .id();
    advance(&mut app, Duration::ZERO);

    // Everything at and before the halfway mark, and nothing beyond it.
    let mut expected_forward = forward_two_stage_subjects(members);
    expected_forward.truncate(7);
    assert_eq!(drained_subjects(&mut app), expected_forward);

    // Holding the same movement repeats no boundary.
    advance(&mut app, Duration::ZERO);
    assert!(
        drained_events(&mut app).is_empty(),
        "an unchanged driver position crosses nothing",
    );

    let backward = [
        FoldEventSubject::MemberEnded {
            member: members[1],
            stage:  1,
        },
        FoldEventSubject::StageEnded(1),
        FoldEventSubject::StageBegan(0),
        FoldEventSubject::MemberBegan {
            member: members[0],
            stage:  0,
        },
    ];
    app.world_mut()
        .entity_mut(driver)
        .insert(movement_toward(0.25, SequenceDirection::Backward));
    advance(&mut app, Duration::ZERO);
    assert_eq!(drained_subjects(&mut app), backward);

    // The same range travelled again emits the same boundaries again.
    app.world_mut().entity_mut(driver).insert(movement(0.75));
    advance(&mut app, Duration::ZERO);
    assert_eq!(drained_subjects(&mut app), expected_forward[3..].to_vec());

    app.world_mut()
        .entity_mut(driver)
        .insert(movement_toward(0.25, SequenceDirection::Backward));
    advance(&mut app, Duration::ZERO);
    assert_eq!(drained_subjects(&mut app), backward);
}

#[test]
fn a_multi_wrap_traversal_emits_every_crossing_of_every_repetition() {
    let mut app = recording_fold_app();
    let (sequence, members) = two_stage_sequence(&mut app);
    let crossings = valid(
        "two whole-sequence crossings",
        RangeCrossings::try_new([
            RangeCrossing::new(RangeEdge::End, SequenceDirection::Forward),
            RangeCrossing::new(RangeEdge::Start, SequenceDirection::Forward),
        ]),
    );
    let wrapping = valid(
        "twice-wrapped movement",
        SequenceMovement::try_new(
            valid(
                "movement position",
                SequencePosition::try_new(HALFWAY_INPUT),
            ),
            SequenceDirection::Forward,
            2,
            crossings,
        ),
    );
    app.world_mut()
        .spawn((SequenceDriver::new(sequence), wrapping));
    advance(&mut app, Duration::ZERO);

    let emitted = drained_subjects(&mut app);
    let ledger = forward_two_stage_subjects(members);
    // Two complete repetitions of the ten-record ledger, then the seven
    // records up to and including the halfway mark.
    let mut expected = ledger.clone();
    expected.extend(ledger.clone());
    expected.extend(ledger[..7].iter().copied());
    assert_eq!(emitted.len(), 27);
    assert_eq!(emitted, expected);
}

#[test]
fn zero_duration_and_all_zero_sequences_emit_adjacent_pairs_and_distinct_endpoints() {
    let mut app = recording_fold_app();
    let (_, members) = quarter_turn_members(&mut app, 2);
    let sequence = app
        .world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::snap())
                .stage(FoldStage::from(fold_group(&members)))
                .build(),
        )
        .id();
    advance(&mut app, Duration::ZERO);
    let _ = drained_events(&mut app);

    play_forward(&mut app, sequence, Duration::ZERO);

    let emitted = drained_events(&mut app);
    assert_eq!(
        subjects_of(&emitted),
        [
            FoldEventSubject::StageBegan(0),
            FoldEventSubject::MemberBegan {
                member: members[0],
                stage:  0,
            },
            FoldEventSubject::MemberEnded {
                member: members[0],
                stage:  0,
            },
            FoldEventSubject::MemberBegan {
                member: members[1],
                stage:  0,
            },
            FoldEventSubject::MemberEnded {
                member: members[1],
                stage:  0,
            },
            FoldEventSubject::StageEnded(0),
            FoldEventSubject::EndpointReached(FoldEndpoint::Base),
            FoldEventSubject::EndpointReached(FoldEndpoint::Folded),
        ],
        "a zero-duration member keeps its begin and end adjacent, in group order",
    );

    // Every authored time is zero, so only the explicit normalized place still
    // separates the two endpoints.
    let endpoints = emitted
        .iter()
        .filter(|event| matches!(event.subject, FoldEventSubject::EndpointReached(_)))
        .collect::<Vec<_>>();
    assert_eq!(endpoints.len(), 2);
    for endpoint in &endpoints {
        assert_eq!(endpoint.timing.elapsed(), SequenceTime::ZERO);
        assert_eq!(endpoint.timing.total(), SequenceTime::ZERO);
    }
    assert_close(
        f64::from(endpoints[0].timing.position().normalized()),
        0.0,
        "the base endpoint holds the sequence start",
    );
    assert_close(
        f64::from(endpoints[1].timing.position().normalized()),
        1.0,
        "the folded endpoint holds the sequence end",
    );
}

#[test]
fn a_hold_and_invalid_movement_emit_nothing() {
    let mut app = recording_fold_app();
    let (sequence, _) = two_stage_sequence(&mut app);

    // Idle, with no journey and no producer.
    advance(&mut app, SECOND);
    assert!(
        drained_events(&mut app).is_empty(),
        "an idle sequence crosses nothing",
    );

    play_forward(&mut app, sequence, SECOND);
    let _ = drained_events(&mut app);
    command(
        &mut app,
        sequence,
        SequenceOwner::NativePlayback,
        SequenceCommand::Pause,
    );
    advance(&mut app, SECOND);
    assert!(
        drained_events(&mut app).is_empty(),
        "a paused journey crosses nothing",
    );

    // A producer whose movement contradicts local position mutates nothing, so
    // it crosses nothing.
    command(
        &mut app,
        sequence,
        SequenceOwner::NativePlayback,
        SequenceCommand::Cancel,
    );
    app.world_mut()
        .spawn((SequenceDriver::new(sequence), movement(0.25)));
    let warnings = WarnLog::default();
    logged_advance(&warnings, &mut app, Duration::ZERO);
    assert_eq!(
        warnings.naming(INVALID_MOVEMENT),
        1,
        "a movement that contradicts local position is reported",
    );
    assert!(
        drained_events(&mut app).is_empty(),
        "invalid movement crosses nothing",
    );
}

/// Every whole stage identity one run's `FoldStageBegin` events carried.
///
/// The recording observers keep only `SequenceStageId::ordinal`, so this is
/// what compares an event's identity against the authored one a
/// `SequenceScope::Stage` is built from — revision included.
#[derive(Resource, Default)]
struct BegunStageIdentities(Vec<SequenceStageId>);

fn record_begun_stage_identity(
    began: On<FoldStageBegin>,
    mut identities: ResMut<BegunStageIdentities>,
) {
    identities.0.push(began.stage);
}

#[test]
fn a_stage_event_carries_the_whole_authored_stage_identity() {
    let mut app = recording_fold_app();
    app.init_resource::<BegunStageIdentities>()
        .add_observer(record_begun_stage_identity);
    let (sequence, _) = two_stage_sequence(&mut app);

    // One seek three quarters of the way through begins both stages.
    app.world_mut()
        .spawn((SequenceDriver::new(sequence), movement(0.75)));
    advance(&mut app, Duration::ZERO);

    assert_eq!(
        app.world().resource::<BegunStageIdentities>().0,
        [stage_id(&app, sequence, 0), stage_id(&app, sequence, 1)],
        "a stage event carries the identity a stage scope selects, not just an ordinal",
    );
}

#[test]
fn stage_output_overshoot_leaves_every_boundary_event_unchanged() {
    let (authored, authored_members) = {
        let mut app = recording_fold_app();
        let (sequence, members) = two_stage_sequence(&mut app);
        app.world_mut()
            .spawn((SequenceDriver::new(sequence), movement(0.75)));
        advance(&mut app, Duration::ZERO);
        let emitted = drained_events(&mut app);
        assert_close(
            eased_of(&app, sequence, members[1]),
            HALFWAY,
            "authored linear easing reaches half its travel",
        );
        (emitted, members)
    };

    let mut app = recording_fold_app();
    let (sequence, members) = two_stage_sequence(&mut app);
    let overshooting = overshooting_curve();
    let second_stage = stage_id(&app, sequence, 1);
    app.world_mut().spawn((
        SequenceDriver::new(sequence),
        SequenceEvaluation::new(
            SequenceScope::Stage(second_stage),
            SequenceEasing::ReplacedBy(Easing::Curve(overshooting)),
        ),
        movement(0.75),
    ));
    advance(&mut app, Duration::ZERO);

    let overshot = drained_events(&mut app);
    assert_close(
        eased_of(&app, sequence, members[1]),
        1.5,
        "the stage output curve carries the member past its target",
    );
    assert_eq!(
        cross_app_boundaries(&overshot, &members),
        cross_app_boundaries(&authored, &authored_members),
        "eased output never reaches a boundary event",
    );
    for (overshot, authored) in overshot.iter().zip(&authored) {
        assert_eq!(overshot.timing, authored.timing);
        assert_eq!(overshot.direction, authored.direction);
    }
}

/// The secondary animation one member observer starts from a fold boundary.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct SecondaryAnimation {
    elapsed:  Duration,
    duration: Duration,
}

/// Starts one secondary animation per member that began travelling.
///
/// This is the documented observer pattern: the event carries the exact place
/// of the member's own boundary and the timing resolved for that member, and
/// the sequence's position is already the updated one, so the animation starts
/// at the travel the fold already passed rather than at zero. Nothing here
/// retains a second progress value.
fn start_secondary_animation(
    began: On<FoldMemberBegin>,
    mut commands: Commands,
    playbacks: Query<&FoldSequencePlayback>,
) {
    let Ok(playback) = playbacks.get(began.arrangement) else {
        return;
    };
    commands.entity(began.member).insert(SecondaryAnimation {
        elapsed:  member_catch_up(began.timing, playback.position()),
        duration: began.member_timing.duration,
    });
}

/// Returns how far past a crossed boundary the sequence already stands.
fn member_catch_up(timing: FoldEventTiming, position: SequencePosition) -> Duration {
    let travelled = f64::from(position.normalized()) - f64::from(timing.position().normalized());
    Duration::from_secs_f64(travelled.abs() * timing.total().as_seconds_f64())
}

#[test]
fn a_member_observer_derives_secondary_animation_catch_up_from_event_timing() {
    let mut app = recording_fold_app();
    app.add_observer(start_secondary_animation);
    let (sequence, members) = two_stage_sequence(&mut app);

    // One seek lands three quarters of the way through a two-second sequence,
    // so the second stage's member began a quarter of a sequence ago.
    app.world_mut()
        .spawn((SequenceDriver::new(sequence), movement(0.75)));
    advance(&mut app, Duration::ZERO);

    assert_eq!(
        app.world().get::<SecondaryAnimation>(members[1]).copied(),
        Some(SecondaryAnimation {
            elapsed:  Duration::from_millis(500),
            duration: SECOND,
        }),
        "a member that began mid-seek catches up to the current position",
    );
    assert_eq!(
        app.world().get::<SecondaryAnimation>(members[0]).copied(),
        Some(SecondaryAnimation {
            elapsed:  Duration::from_millis(1500),
            duration: SECOND,
        }),
        "a member whose whole segment already passed catches up past its own extent",
    );
}
