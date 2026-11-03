//! Still-frame work avoidance for anchor resolution and fold playback.

#![allow(
    clippy::expect_used,
    reason = "tests should fail immediately when required fixture state is absent"
)]

use std::time::Duration;

use bevy::app::App;
use bevy::app::PostUpdate;
use bevy::app::TaskPoolPlugin;
use bevy::app::Update;
use bevy::asset::AssetPlugin;
use bevy::curve::EaseFunction;
use bevy::ecs::change_detection::DetectChanges;
use bevy::ecs::schedule::IntoScheduleConfigs as _;
use bevy::ecs::system::Query;
use bevy::ecs::system::Res;
use bevy::ecs::system::ResMut;
use bevy::ecs::system::RunSystemOnce as _;
use bevy::math::Vec3;
use bevy::prelude::ChildOf;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Ref;
use bevy::prelude::Resource;
use bevy::prelude::Transform;
use bevy::time::Time;
use bevy::time::Virtual;
use bevy::transform::TransformPlugin;
use bevy::transform::TransformSystems;
use hana_valence::AnchorFrame;
use hana_valence::AnchorPose;
use hana_valence::AnchorResolveDiagnostics;
use hana_valence::AnchorSite;
use hana_valence::AnchorSystems;
use hana_valence::AnchoredTo;
use hana_valence::Angle;
use hana_valence::Displacement;
use hana_valence::Edge;
use hana_valence::FoldCommands;
use hana_valence::FoldPlugin;
use hana_valence::FoldSequenceBuilder;
use hana_valence::FoldSequencePlayback;
use hana_valence::FoldStage;
use hana_valence::FoldSystems;
use hana_valence::FoldTiming;
use hana_valence::Hinge;
use hana_valence::Orientation;
use hana_valence::Position;
use hana_valence::ResolvedAnchorGeometry;
use hana_valence::ResolvedAnchorWorld;
use hana_valence::SequenceCommand;
use hana_valence::SequenceCommandResponse;
use hana_valence::SequenceOwner;
use hana_valence::resolve_anchors;

const STEP: Duration = Duration::from_millis(100);

#[derive(Default, Resource)]
struct AnchorPassProbe {
    diagnostics_changed:   bool,
    changed_anchor_worlds: usize,
}

fn observe_anchor_pass(
    diagnostics: Res<AnchorResolveDiagnostics>,
    changed_anchor_worlds: Query<(), bevy::ecs::query::Changed<ResolvedAnchorWorld>>,
    mut probe: ResMut<AnchorPassProbe>,
) {
    probe.diagnostics_changed = diagnostics.is_changed();
    probe.changed_anchor_worlds = changed_anchor_worlds.iter().count();
}

fn anchor_app() -> App {
    let mut app = App::new();
    app.add_plugins((TaskPoolPlugin::default(), TransformPlugin))
        .insert_resource(AnchorResolveDiagnostics::default())
        .init_resource::<AnchorPassProbe>()
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
        .add_systems(
            PostUpdate,
            (
                resolve_anchors.in_set(AnchorSystems::Resolve),
                observe_anchor_pass.after(AnchorSystems::Resolve),
            ),
        );
    app
}

fn center_geometry() -> ResolvedAnchorGeometry {
    let frame = AnchorFrame::try_new(Position::default(), Orientation::default())
        .expect("the origin is a valid anchor frame");
    ResolvedAnchorGeometry::try_new([(AnchorSite::Center, frame)], [])
        .expect("one center frame is valid geometry")
}

fn spawn_anchor_target(app: &mut App, translation: Vec3) -> Entity {
    let transform = Transform::from_translation(translation);
    app.world_mut()
        .spawn((
            center_geometry(),
            transform,
            GlobalTransform::from(transform),
        ))
        .id()
}

fn spawn_anchored_source(app: &mut App, target: Entity) -> Entity {
    app.world_mut()
        .spawn((
            center_geometry(),
            Transform::default(),
            GlobalTransform::default(),
            ResolvedAnchorWorld::default(),
            AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
        ))
        .id()
}

fn settle_anchors(app: &mut App) {
    app.update();
    app.update();
}

fn assert_anchor_pass_ran(app: &App) {
    let probe = app.world().resource::<AnchorPassProbe>();
    assert!(
        probe.diagnostics_changed || probe.changed_anchor_worlds > 0,
        "the resolver should mutate its diagnostics frame or an anchor-world cache",
    );
}

#[test]
fn a_still_frame_does_no_anchor_work() {
    let mut app = anchor_app();
    let target = spawn_anchor_target(&mut app, Vec3::ZERO);
    spawn_anchored_source(&mut app, target);
    settle_anchors(&mut app);

    app.update();

    let probe = app.world().resource::<AnchorPassProbe>();
    assert!(!probe.diagnostics_changed);
    assert_eq!(probe.changed_anchor_worlds, 0);
}

#[test]
fn a_changed_anchor_pose_runs_the_pass_on_that_frame() {
    let mut app = anchor_app();
    let target = spawn_anchor_target(&mut app, Vec3::new(2.0, 0.0, 0.0));
    let source = spawn_anchored_source(&mut app, target);
    settle_anchors(&mut app);
    app.world_mut().entity_mut(source).insert(AnchorPose {
        translation: Displacement::from(Vec3::new(3.0, 0.0, 0.0)),
        ..AnchorPose::default()
    });

    app.update();

    assert_anchor_pass_ran(&app);
    assert_eq!(
        app.world()
            .get::<Transform>(source)
            .expect("the anchored source keeps its transform")
            .translation,
        Vec3::new(5.0, 0.0, 0.0),
    );
}

#[test]
fn a_child_anchored_elsewhere_follows_its_parents_pose_change() {
    let mut app = anchor_app();
    let root = spawn_anchor_target(&mut app, Vec3::new(10.0, 0.0, 0.0));
    let target = spawn_anchored_source(&mut app, root);
    let parent = spawn_anchored_source(&mut app, root);
    app.world_mut().entity_mut(parent).insert(AnchorPose {
        translation: Displacement::from(Vec3::new(2.0, 0.0, 0.0)),
        ..AnchorPose::default()
    });
    let child = spawn_anchored_source(&mut app, target);
    app.world_mut().entity_mut(child).insert(ChildOf(parent));
    settle_anchors(&mut app);

    app.world_mut().entity_mut(parent).insert(AnchorPose {
        translation: Displacement::from(Vec3::new(4.0, 0.0, 0.0)),
        ..AnchorPose::default()
    });
    app.update();

    assert_eq!(
        app.world()
            .get::<GlobalTransform>(child)
            .expect("the anchored child keeps its global transform")
            .translation(),
        Vec3::new(10.0, 0.0, 0.0),
    );
}

#[test]
fn a_moved_root_resolves_its_whole_anchor_chain_in_one_frame() {
    let mut app = anchor_app();
    let root = spawn_anchor_target(&mut app, Vec3::ZERO);
    let middle = spawn_anchored_source(&mut app, root);
    let leaf = spawn_anchored_source(&mut app, middle);
    settle_anchors(&mut app);
    app.world_mut()
        .entity_mut(root)
        .insert(GlobalTransform::from_translation(Vec3::new(7.0, 0.0, 0.0)));

    app.update();

    assert_anchor_pass_ran(&app);
    for entity in [middle, leaf] {
        assert_eq!(
            app.world()
                .get::<Transform>(entity)
                .expect("every anchored member keeps its transform")
                .translation,
            Vec3::new(7.0, 0.0, 0.0),
        );
    }
}

#[test]
fn a_despawned_target_runs_the_anchor_pass() {
    let mut app = anchor_app();
    let target = spawn_anchor_target(&mut app, Vec3::ZERO);
    spawn_anchored_source(&mut app, target);
    settle_anchors(&mut app);
    app.world_mut().entity_mut(target).despawn();

    app.update();

    assert_anchor_pass_ran(&app);
}

#[test]
fn a_new_anchored_entity_runs_the_anchor_pass() {
    let mut app = anchor_app();
    let target = spawn_anchor_target(&mut app, Vec3::new(4.0, 0.0, 0.0));
    settle_anchors(&mut app);
    let source = spawn_anchored_source(&mut app, target);

    app.update();

    assert_anchor_pass_ran(&app);
    assert_eq!(
        app.world()
            .get::<Transform>(source)
            .expect("the new anchored source keeps its transform")
            .translation,
        Vec3::new(4.0, 0.0, 0.0),
    );
}

#[derive(Default, Resource)]
struct PlaybackProbe {
    changed: usize,
}

fn observe_playback(playbacks: Query<Ref<FoldSequencePlayback>>, mut probe: ResMut<PlaybackProbe>) {
    probe.changed = playbacks.iter().filter(DetectChanges::is_changed).count();
}

fn playback_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        TaskPoolPlugin::default(),
        AssetPlugin::default(),
        FoldPlugin,
    ))
    .insert_resource(Time::<Virtual>::default())
    .init_resource::<PlaybackProbe>()
    .add_systems(Update, observe_playback.after(FoldSystems::Advance));
    app
}

fn hinge_geometry() -> ResolvedAnchorGeometry {
    let frames = [
        (
            AnchorSite::Vertex(0),
            AnchorFrame::try_new(Position::from(Vec3::NEG_Y), Orientation::default())
                .expect("the first hinge frame is finite"),
        ),
        (
            AnchorSite::Vertex(1),
            AnchorFrame::try_new(Position::from(Vec3::Y), Orientation::default())
                .expect("the second hinge frame is finite"),
        ),
    ];
    ResolvedAnchorGeometry::try_new(
        frames,
        [Edge {
            start: AnchorSite::Vertex(0),
            end:   AnchorSite::Vertex(1),
        }],
    )
    .expect("two distinct frames make valid hinge geometry")
}

fn spawn_fold(app: &mut App, duration: Duration) -> Entity {
    let member = app
        .world_mut()
        .spawn((
            hinge_geometry(),
            Hinge::try_new(
                Edge {
                    start: AnchorSite::Vertex(0),
                    end:   AnchorSite::Vertex(1),
                },
                Angle::default(),
                Angle::from_radians(std::f32::consts::FRAC_PI_2)
                    .expect("a quarter turn is a finite angle"),
                Displacement::default(),
            )
            .expect("the fixture hinge is valid"),
        ))
        .id();
    app.world_mut()
        .spawn(
            FoldSequenceBuilder::new(FoldTiming::new(duration, EaseFunction::Linear))
                .stage(FoldStage::from(member))
                .build(),
        )
        .id()
}

fn issue_play(
    bevy::ecs::system::In(sequence): bevy::ecs::system::In<Entity>,
    mut commands: FoldCommands,
) {
    let response = commands.apply(
        sequence,
        SequenceOwner::NativePlayback,
        SequenceCommand::Play,
    );
    assert!(
        matches!(response, SequenceCommandResponse::Permitted(_)),
        "the native play command should be accepted",
    );
}

fn advance(app: &mut App, delta: Duration) {
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .advance_by(delta);
    app.update();
}

#[test]
fn an_idle_playback_is_not_ticked() {
    let mut app = playback_app();
    spawn_fold(&mut app, STEP * 2);
    advance(&mut app, Duration::ZERO);

    advance(&mut app, Duration::ZERO);

    assert_eq!(app.world().resource::<PlaybackProbe>().changed, 0);
}

#[test]
fn a_native_journey_ticks_until_it_ends_then_stops() {
    let mut app = playback_app();
    let sequence = spawn_fold(&mut app, STEP * 3);
    advance(&mut app, Duration::ZERO);
    app.world_mut()
        .run_system_once_with(issue_play, sequence)
        .expect("issuing the native play command should run");

    for step in 1..=3 {
        advance(&mut app, STEP);
        assert_eq!(
            app.world().resource::<PlaybackProbe>().changed,
            1,
            "the native journey should tick on movement step {step}",
        );
    }
    assert!(
        !app.world()
            .get::<FoldSequencePlayback>(sequence)
            .expect("the retained playback stays present")
            .is_playing(),
    );

    advance(&mut app, STEP);

    assert_eq!(app.world().resource::<PlaybackProbe>().changed, 0);
}
