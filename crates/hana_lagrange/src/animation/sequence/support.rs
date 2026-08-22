//! Shared fixtures for the retained camera sequence tests.

use std::any::TypeId;
use std::time::Duration;

use bevy::camera::CameraProjection;
use bevy::camera::RenderTarget;
use bevy::camera::SubCameraView;
use bevy::ecs::reflect::ReflectEvent;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::math::curve::easing::EaseFunction;
use bevy::prelude::Add;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::ButtonInput;
use bevy::prelude::Camera;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::In;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Mat4;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MinimalPlugins;
use bevy::prelude::MouseButton;
use bevy::prelude::On;
use bevy::prelude::OrthographicProjection;
use bevy::prelude::PerspectiveProjection;
use bevy::prelude::PreUpdate;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Remove;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Vec3A;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::reflect::ReflectFromReflect;
use bevy::reflect::TypePath;
use bevy::reflect::TypeRegistry;
use bevy::window::WindowRef;
use bevy_kana::DriverRestoration;
use bevy_kana::RangeCrossings;
use bevy_kana::SequenceCommand;
use bevy_kana::SequenceCommandResponse;
use bevy_kana::SequenceDirection;
use bevy_kana::SequenceDriver;
use bevy_kana::SequenceDriverReleased;
use bevy_kana::SequenceMovement;
use bevy_kana::SequenceOwner;
use bevy_kana::SequenceOwnership;
use bevy_kana::SequencePosition;
use bevy_kana::SequenceSourceState;
use bevy_kana::SequenceStageId;
use bevy_kana::SequenceStageSpan;
use bevy_kana::SequenceStages;
use bevy_kana::SequenceStagesRevision;
use bevy_kana::SequenceTime;

use super::CameraSequence;
use super::controller_installation::*;
use super::playback::*;
use super::request::*;
use crate::AnimationReason;
use crate::AnimationSource;
use crate::CameraBasis;
use crate::CameraCommands;
use crate::CameraEventTiming;
use crate::CameraInputInterruptBehavior;
use crate::CameraPlaybackObservation;
use crate::CameraRequestPreparationError;
use crate::CurrentFitTarget;
use crate::FreeCam;
use crate::FreeCamInputMode;
use crate::FreeCamManualInputWriter;
use crate::InputIntent;
use crate::LagrangePlugin;
use crate::LookAngles;
use crate::ManualInputSource;
use crate::OrbitCam;
use crate::OrbitCamInputMode;
use crate::OrbitCamManualInputWriter;
use crate::PlayAnimation;
use crate::Position;
use crate::Radius;
use crate::Roll;
use crate::ZoomReason;
use crate::animation::AnimationPlugin;
use crate::animation::FreeCamRollTarget;
use crate::animation::events::AnimationBegin;
use crate::animation::events::AnimationEnd;
use crate::animation::events::AnimationRejected;
use crate::animation::events::AnimationRejectionReason;
use crate::animation::events::CameraMoveBegin;
use crate::animation::events::CameraMoveEnd;
use crate::animation::lifecycle::FreeFlightControllerOverrideRestoration;
use crate::animation::lifecycle::OrbitControllerOverrideRestoration;
use crate::animation::queue::CameraMove;
use crate::fit::FitPlugin;
use crate::fit::ZoomBegin;
use crate::fit::ZoomEnd;
use crate::operation::Focus;
use crate::operation::OrbitAngles;
use crate::system_sets::CameraInputPhase;

pub(super) type TestResult = Result<(), &'static str>;

pub(super) const FIRST_MOVE_MILLIS: u64 = 250;

pub(super) const SECOND_MOVE_MILLIS: u64 = 0;

pub(super) const THIRD_MOVE_MILLIS: u64 = 750;

pub(super) const CAMERA_COMMANDS: [SequenceCommand; 7] = [
    SequenceCommand::Play,
    SequenceCommand::PlayBackward,
    SequenceCommand::Pause,
    SequenceCommand::Resume,
    SequenceCommand::Cancel,
    SequenceCommand::Step,
    SequenceCommand::StepBackward,
];

#[derive(Component)]
pub(super) struct OrbitInterruptionTestCamera;

#[derive(Component)]
pub(super) struct FreeInterruptionTestCamera;

#[derive(Resource, Default)]
pub(super) struct AnimationClosureCounts {
    pub(super) completed: usize,
    pub(super) cancelled: usize,
}

#[derive(Resource, Default)]
pub(super) struct LifecycleEventOrder(pub(super) Vec<&'static str>);

#[derive(Resource, Default)]
pub(super) struct CameraBoundaryEventOrder(pub(super) Vec<(&'static str, Duration)>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CameraEventOutcome {
    Completed,
    Cancelled,
}

/// One ordered record of every public camera event emitted by a retained journey.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum CameraEventTraceItem {
    ZoomBegin {
        camera: Entity,
        target: Entity,
    },
    AnimationBegin {
        camera:    Entity,
        source:    AnimationSource,
        target:    Option<Entity>,
        owner:     SequenceOwner,
        direction: SequenceDirection,
        timing:    CameraEventTiming,
    },
    CameraMoveBegin {
        camera:           Entity,
        stage_id:         SequenceStageId,
        owner:            SequenceOwner,
        direction:        SequenceDirection,
        boundary_elapsed: SequenceTime,
        timing:           CameraEventTiming,
    },
    CameraMoveEnd {
        camera:           Entity,
        stage_id:         SequenceStageId,
        owner:            SequenceOwner,
        direction:        SequenceDirection,
        boundary_elapsed: SequenceTime,
        timing:           CameraEventTiming,
    },
    AnimationEnd {
        camera:    Entity,
        source:    AnimationSource,
        target:    Option<Entity>,
        owner:     SequenceOwner,
        direction: SequenceDirection,
        timing:    CameraEventTiming,
        outcome:   CameraEventOutcome,
    },
    ZoomEnd {
        camera:  Entity,
        target:  Entity,
        outcome: CameraEventOutcome,
    },
}

#[derive(Resource, Default)]
pub(super) struct CameraEventTrace(pub(super) Vec<CameraEventTraceItem>);

#[derive(Resource, Default)]
pub(super) struct ControllerOverrideTransitionCounts {
    pub(super) captured: usize,
    pub(super) released: usize,
}

#[derive(Resource, Default)]
pub(super) struct AnimationBeginMetadata(pub(super) Vec<(AnimationSource, Option<Entity>)>);

#[derive(Resource, Default)]
pub(super) struct ZoomBeginMetadata(pub(super) Vec<(Entity, f32, Duration, EaseFunction)>);

#[derive(Resource, Default)]
pub(super) struct ObservedDriverRestorations(pub(super) Vec<DriverRestoration>);

#[derive(Resource)]
pub(super) struct SelectedDriverDespawn(pub(super) Entity);

pub(super) fn camera_sequence_test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, LagrangePlugin))
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<AccumulatedMouseMotion>()
        .init_resource::<AccumulatedMouseScroll>()
        .init_resource::<ControllerOverrideTransitionCounts>()
        .add_observer(count_controller_override_capture)
        .add_observer(count_controller_override_release)
        .add_observer(count_free_controller_override_capture)
        .add_observer(count_free_controller_override_release);
    app.finish();
    app
}

pub(super) fn count_controller_override_capture(
    _: On<Add, OrbitControllerOverrideRestoration>,
    mut counts: ResMut<ControllerOverrideTransitionCounts>,
) {
    counts.captured += 1;
}

pub(super) fn count_controller_override_release(
    _: On<Remove, OrbitControllerOverrideRestoration>,
    mut counts: ResMut<ControllerOverrideTransitionCounts>,
) {
    counts.released += 1;
}

pub(super) fn count_free_controller_override_capture(
    _: On<Add, FreeFlightControllerOverrideRestoration>,
    mut counts: ResMut<ControllerOverrideTransitionCounts>,
) {
    counts.captured += 1;
}

pub(super) fn count_free_controller_override_release(
    _: On<Remove, FreeFlightControllerOverrideRestoration>,
    mut counts: ResMut<ControllerOverrideTransitionCounts>,
) {
    counts.released += 1;
}

pub(super) fn record_animation_begin_metadata(
    begin: On<AnimationBegin>,
    mut metadata: ResMut<AnimationBeginMetadata>,
) {
    metadata.0.push((begin.source, begin.target));
}

pub(super) fn record_zoom_begin_metadata(
    begin: On<ZoomBegin>,
    mut metadata: ResMut<ZoomBeginMetadata>,
) {
    metadata
        .0
        .push((begin.target, begin.margin, begin.duration, begin.easing));
}

pub(super) fn record_driver_restoration(
    released: On<SequenceDriverReleased>,
    mut restorations: ResMut<ObservedDriverRestorations>,
) {
    restorations.0.push(released.restoration);
}

pub(super) fn interruption_test_app() -> App {
    let mut app = camera_sequence_test_app();
    app.init_resource::<AnimationClosureCounts>().add_systems(
        PreUpdate,
        (
            write_orbit_interruption_input,
            write_free_interruption_input,
        )
            .in_set(CameraInputPhase::WriteManual),
    );
    app
}

pub(super) fn write_orbit_interruption_input(
    mut writer: OrbitCamManualInputWriter,
    cameras: Query<Entity, With<OrbitInterruptionTestCamera>>,
) {
    for camera in &cameras {
        if let Ok(mut input) = writer.get_mut(camera, ManualInputSource::manual()) {
            input.orbit(Vec2::X);
        }
    }
}

pub(super) fn write_free_interruption_input(
    mut writer: FreeCamManualInputWriter,
    cameras: Query<Entity, With<FreeInterruptionTestCamera>>,
) {
    for camera in &cameras {
        if let Ok(mut input) = writer.get_mut(camera, ManualInputSource::manual()) {
            input.translate(Vec3::X);
        }
    }
}

pub(super) fn count_animation_closures(world: &mut World, camera: Entity) {
    world.entity_mut(camera).observe(
        |event: On<AnimationEnd>, mut counts: ResMut<AnimationClosureCounts>| match event.reason {
            AnimationReason::Completed => counts.completed += 1,
            AnimationReason::Cancelled { .. } => counts.cancelled += 1,
        },
    );
}

pub(super) fn record_lifecycle_order(world: &mut World, camera: Entity) {
    world.entity_mut(camera).observe(
        |_: On<AnimationBegin>, mut order: ResMut<LifecycleEventOrder>| {
            order.0.push("begin");
        },
    );
    world.entity_mut(camera).observe(
        |event: On<AnimationEnd>, mut order: ResMut<LifecycleEventOrder>| {
            if matches!(event.reason, AnimationReason::Cancelled { .. }) {
                order.0.push("cancel");
            } else {
                order.0.push("complete");
            }
        },
    );
}

pub(super) fn record_camera_boundary_order(world: &mut World, camera: Entity) {
    world.entity_mut(camera).observe(
        |event: On<CameraMoveBegin>, mut order: ResMut<CameraBoundaryEventOrder>| {
            order.0.push(("begin", event.camera_move.duration()));
        },
    );
    world.entity_mut(camera).observe(
        |event: On<CameraMoveEnd>, mut order: ResMut<CameraBoundaryEventOrder>| {
            order.0.push(("end", event.camera_move.duration()));
        },
    );
}

pub(super) fn record_trace_zoom_begin(begin: On<ZoomBegin>, mut trace: ResMut<CameraEventTrace>) {
    trace.0.push(CameraEventTraceItem::ZoomBegin {
        camera: begin.camera,
        target: begin.target,
    });
}

pub(super) fn record_trace_animation_begin(
    begin: On<AnimationBegin>,
    mut trace: ResMut<CameraEventTrace>,
) {
    trace.0.push(CameraEventTraceItem::AnimationBegin {
        camera:    begin.camera,
        source:    begin.source,
        target:    begin.target,
        owner:     begin.owner,
        direction: begin.direction,
        timing:    begin.timing,
    });
}

pub(super) fn record_trace_camera_move_begin(
    begin: On<CameraMoveBegin>,
    mut trace: ResMut<CameraEventTrace>,
) {
    trace.0.push(CameraEventTraceItem::CameraMoveBegin {
        camera:           begin.camera,
        stage_id:         begin.stage_id,
        owner:            begin.owner,
        direction:        begin.direction,
        boundary_elapsed: begin.boundary_elapsed,
        timing:           begin.timing,
    });
}

pub(super) fn record_trace_camera_move_end(
    end: On<CameraMoveEnd>,
    mut trace: ResMut<CameraEventTrace>,
) {
    trace.0.push(CameraEventTraceItem::CameraMoveEnd {
        camera:           end.camera,
        stage_id:         end.stage_id,
        owner:            end.owner,
        direction:        end.direction,
        boundary_elapsed: end.boundary_elapsed,
        timing:           end.timing,
    });
}

pub(super) fn record_trace_animation_end(
    end: On<AnimationEnd>,
    mut trace: ResMut<CameraEventTrace>,
) {
    trace.0.push(CameraEventTraceItem::AnimationEnd {
        camera:    end.camera,
        source:    end.source,
        target:    end.target,
        owner:     end.owner,
        direction: end.direction,
        timing:    end.timing,
        outcome:   match end.reason {
            AnimationReason::Completed => CameraEventOutcome::Completed,
            AnimationReason::Cancelled { .. } => CameraEventOutcome::Cancelled,
        },
    });
}

pub(super) fn record_trace_zoom_end(end: On<ZoomEnd>, mut trace: ResMut<CameraEventTrace>) {
    trace.0.push(CameraEventTraceItem::ZoomEnd {
        camera:  end.camera,
        target:  end.target,
        outcome: match end.reason {
            ZoomReason::Completed => CameraEventOutcome::Completed,
            ZoomReason::Cancelled => CameraEventOutcome::Cancelled,
        },
    });
}

pub(super) fn camera_event_trace_app() -> App {
    let mut app = camera_sequence_test_app();
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<CameraEventTrace>()
        .add_observer(record_trace_zoom_begin)
        .add_observer(record_trace_animation_begin)
        .add_observer(record_trace_camera_move_begin)
        .add_observer(record_trace_camera_move_end)
        .add_observer(record_trace_animation_end)
        .add_observer(record_trace_zoom_end);
    app
}

pub(super) fn despawn_selected_driver_after_movement(world: &mut World) {
    let Some(despawn) = world.remove_resource::<SelectedDriverDespawn>() else {
        return;
    };
    world.despawn(despawn.0);
}

pub(super) fn assert_interruption_result(
    app: &App,
    camera: Entity,
    behavior: CameraInputInterruptBehavior,
) -> TestResult {
    let retained = app
        .world()
        .get::<CameraSequencePlayback>(camera)
        .ok_or("accepted playback was not retained")?;
    let counts = app.world().resource::<AnimationClosureCounts>();
    match behavior {
        CameraInputInterruptBehavior::Ignore => {
            assert_eq!((counts.completed, counts.cancelled), (0, 0));
            assert!(matches!(
                retained.lifecycle,
                CameraPlaybackLifecycleState::Effective {
                    owner: SequenceOwner::NativePlayback,
                }
            ));
        },
        CameraInputInterruptBehavior::Cancel => {
            assert_eq!((counts.completed, counts.cancelled), (0, 1));
            assert_eq!(retained.playback.position(), SequencePosition::START);
            assert_eq!(retained.lifecycle, CameraPlaybackLifecycleState::Dormant);
        },
        CameraInputInterruptBehavior::Complete => {
            assert_eq!((counts.completed, counts.cancelled), (1, 0));
            assert_eq!(retained.playback.position(), SequencePosition::END);
            assert_eq!(retained.lifecycle, CameraPlaybackLifecycleState::Dormant);
        },
    }
    Ok(())
}

pub(super) fn run_orbit_interruption(behavior: CameraInputInterruptBehavior) -> TestResult {
    let mut app = interruption_test_app();
    let camera = app
        .world_mut()
        .spawn((
            OrbitInterruptionTestCamera,
            OrbitCam::default(),
            crate::OrbitCamInput::default(),
            OrbitCamInputMode::Manual,
            crate::input::CameraManual::<crate::OrbitCamKind>::default(),
            Camera::default(),
            RenderTarget::Window(WindowRef::Primary),
            Projection::Perspective(PerspectiveProjection::default()),
            Transform::from_xyz(0.0, 0.0, 10.0),
            crate::CameraInputSurfaceMetrics::camera_view_and_input_surface(
                Vec2::splat(100.0),
                Vec2::splat(100.0),
            ),
            behavior,
        ))
        .id();
    count_animation_closures(app.world_mut(), camera);
    app.world_mut().trigger(PlayAnimation::new(
        camera,
        [move_lasting(Duration::from_secs(1))],
    ));

    app.update();

    assert!(
        !app.world()
            .get::<crate::OrbitCamInput>(camera)
            .is_some_and(InputIntent::has_input)
    );
    assert_interruption_result(&app, camera, behavior)
}

pub(super) fn run_free_interruption(behavior: CameraInputInterruptBehavior) -> TestResult {
    let mut app = interruption_test_app();
    let camera = app
        .world_mut()
        .spawn((
            FreeInterruptionTestCamera,
            FreeCam::default(),
            crate::FreeCamInput::default(),
            FreeCamInputMode::Manual,
            crate::input::CameraManual::<crate::FreeCamKind>::default(),
            CameraBasis::Y_UP,
            Camera::default(),
            Projection::Perspective(PerspectiveProjection::default()),
            Transform::from_xyz(0.0, 0.0, 10.0),
            behavior,
        ))
        .id();
    count_animation_closures(app.world_mut(), camera);
    app.world_mut().trigger(PlayAnimation::new(
        camera,
        [move_lasting(Duration::from_secs(1))],
    ));

    app.update();

    assert!(
        !app.world()
            .get::<crate::FreeCamInput>(camera)
            .is_some_and(InputIntent::has_input)
    );
    assert_interruption_result(&app, camera, behavior)
}

pub(super) fn run_selected_orbit_policy(behavior: CameraInputInterruptBehavior) -> TestResult {
    let mut app = interruption_test_app();
    let camera = app
        .world_mut()
        .spawn((
            OrbitInterruptionTestCamera,
            OrbitCam::default(),
            crate::OrbitCamInput::default(),
            OrbitCamInputMode::Manual,
            crate::input::CameraManual::<crate::OrbitCamKind>::default(),
            Camera::default(),
            RenderTarget::Window(WindowRef::Primary),
            Projection::Perspective(PerspectiveProjection::default()),
            Transform::from_xyz(0.0, 0.0, 10.0),
            crate::CameraInputSurfaceMetrics::camera_view_and_input_surface(
                Vec2::splat(100.0),
                Vec2::splat(100.0),
            ),
            behavior,
            CameraSequence::new(move_lasting(Duration::from_secs(1))),
        ))
        .id();
    count_animation_closures(app.world_mut(), camera);
    let driver = app
        .world_mut()
        .spawn(bevy_kana::SequenceDriver::new(camera))
        .id();

    app.update();

    let retained = app
        .world()
        .get::<CameraSequencePlayback>(camera)
        .ok_or("direct selected playback was not prepared")?;
    assert!(matches!(
        retained.lifecycle,
        CameraPlaybackLifecycleState::Effective {
            owner: SequenceOwner::Driver(selected)
        } if selected == driver
    ));
    assert_eq!(retained.playback.position(), SequencePosition::START);
    assert_eq!(
        (
            app.world().resource::<AnimationClosureCounts>().completed,
            app.world().resource::<AnimationClosureCounts>().cancelled
        ),
        (0, 0)
    );
    assert!(
        !app.world()
            .get::<crate::OrbitCamInput>(camera)
            .is_some_and(InputIntent::has_input)
    );
    Ok(())
}

pub(super) fn observe_camera_commands(
    input: In<Entity>,
    commands: CameraCommands,
) -> (SequenceOwnership, CameraPlaybackObservation) {
    (commands.owner(input.0), commands.observe(input.0))
}

pub(super) fn issue_camera_command(
    input: In<(Entity, SequenceOwner, SequenceCommand)>,
    mut commands: CameraCommands,
) -> SequenceCommandResponse {
    let (camera, issuer, command) = input.0;
    commands.apply(camera, issuer, command)
}

pub(super) fn move_lasting(duration: Duration) -> CameraMove {
    CameraMove::try_to_orbital_look_at(
        Focus(Vec3::ZERO),
        OrbitAngles {
            yaw:   0.25,
            pitch: 0.5,
        },
        Radius(4.0),
        FreeCamRollTarget::InheritPrevious,
        duration,
        EaseFunction::Linear,
    )
    .expect("the authored orbital pose is finite")
}

pub(super) fn three_move_sequence() -> CameraSequence {
    CameraSequence::new(move_lasting(Duration::from_millis(FIRST_MOVE_MILLIS)))
        .then(move_lasting(Duration::from_millis(SECOND_MOVE_MILLIS)))
        .then(move_lasting(Duration::from_millis(THIRD_MOVE_MILLIS)))
}

pub(super) fn camera_prepared_for_native_command(
    command: SequenceCommand,
) -> Result<(App, Entity), &'static str> {
    let mut app = camera_sequence_test_app();
    let camera = app
        .world_mut()
        .spawn((
            orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0),
            CameraSequence::new(move_lasting(Duration::from_secs(1))),
        ))
        .id();
    app.update();

    if matches!(
        command,
        SequenceCommand::PlayBackward | SequenceCommand::StepBackward
    ) {
        let movement = SequenceMovement::try_new(
            SequencePosition::END,
            SequenceDirection::Forward,
            0,
            RangeCrossings::NONE,
        )
        .map_err(|_| "the endpoint fixture movement is valid")?;
        let driver = app
            .world_mut()
            .spawn((bevy_kana::SequenceDriver::new(camera), movement))
            .id();
        app.update();
        app.world_mut().despawn(driver);
        app.update();
    }

    let preconditions: &[SequenceCommand] = match command {
        SequenceCommand::Pause | SequenceCommand::Cancel => &[SequenceCommand::Play],
        SequenceCommand::Resume => &[SequenceCommand::Play, SequenceCommand::Pause],
        SequenceCommand::Play
        | SequenceCommand::PlayBackward
        | SequenceCommand::Step
        | SequenceCommand::StepBackward => &[],
    };
    for precondition in preconditions {
        assert_eq!(
            app.world_mut()
                .run_system_cached_with(
                    issue_camera_command,
                    (camera, SequenceOwner::NativePlayback, *precondition),
                )
                .map_err(|_| "native command precondition did not run")?,
            SequenceCommandResponse::Permitted(bevy_kana::SequenceCommandOutcome::Applied,)
        );
    }
    Ok((app, camera))
}

pub(super) fn assert_absent_camera_command_state(app: &mut App) -> TestResult {
    let absent = app.world_mut().spawn_empty().id();
    assert_eq!(
        app.world_mut()
            .run_system_cached_with(observe_camera_commands, absent)
            .map_err(|_| "camera observation system did not run")?,
        (
            SequenceOwnership::NoRetainedSequence,
            CameraPlaybackObservation::NoRetainedSequence,
        )
    );
    for command in CAMERA_COMMANDS {
        assert_eq!(
            app.world_mut()
                .run_system_cached_with(
                    issue_camera_command,
                    (absent, SequenceOwner::NativePlayback, command),
                )
                .map_err(|_| "absent camera command system did not run")?,
            SequenceCommandResponse::NoRetainedSequence
        );
    }
    Ok(())
}

pub(super) fn assert_native_camera_command_states() -> TestResult {
    for command in CAMERA_COMMANDS {
        let (mut app, camera) = camera_prepared_for_native_command(command)?;
        let (ownership, observation) = app
            .world_mut()
            .run_system_cached_with(observe_camera_commands, camera)
            .map_err(|_| "native camera observation system did not run")?;
        assert_eq!(
            ownership,
            SequenceOwnership::Retained(SequenceOwner::NativePlayback)
        );
        assert!(matches!(
            observation,
            CameraPlaybackObservation::Retained {
                owner: SequenceOwner::NativePlayback,
                ..
            }
        ));
        assert_eq!(
            app.world_mut()
                .run_system_cached_with(
                    issue_camera_command,
                    (camera, SequenceOwner::NativePlayback, command),
                )
                .map_err(|_| "permitted native command system did not run")?,
            SequenceCommandResponse::Permitted(bevy_kana::SequenceCommandOutcome::Applied,),
            "{command:?} should apply from its prepared fixture state",
        );
        assert_eq!(
            app.world_mut()
                .run_system_cached_with(
                    issue_camera_command,
                    (camera, SequenceOwner::NativePlayback, command),
                )
                .map_err(|_| "repeated native command system did not run")?,
            SequenceCommandResponse::Permitted(bevy_kana::SequenceCommandOutcome::NoChange,),
            "repeating {command:?} should be a permitted no-change",
        );
    }
    Ok(())
}

pub(super) fn assert_selected_driver_command_state(app: &mut App) -> TestResult {
    let driver = app.world_mut().spawn_empty().id();
    let camera = app
        .world_mut()
        .spawn((
            orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0),
            CameraSequence::new(move_lasting(Duration::from_secs(1))),
        ))
        .id();
    app.world_mut()
        .entity_mut(driver)
        .insert(bevy_kana::SequenceDriver::new(camera));
    app.update();
    let (ownership, observation) = app
        .world_mut()
        .run_system_cached_with(observe_camera_commands, camera)
        .map_err(|_| "selected camera observation system did not run")?;
    assert_eq!(
        ownership,
        SequenceOwnership::Retained(SequenceOwner::Driver(driver))
    );
    assert!(matches!(
        observation,
        CameraPlaybackObservation::Retained {
            owner: SequenceOwner::Driver(selected),
            ..
        } if selected == driver
    ));
    for command in CAMERA_COMMANDS {
        assert_eq!(
            app.world_mut()
                .run_system_cached_with(
                    issue_camera_command,
                    (camera, SequenceOwner::NativePlayback, command),
                )
                .map_err(|_| "rejected native command system did not run")?,
            SequenceCommandResponse::Rejected(SequenceOwner::Driver(driver)),
            "{command:?} should name the selected driver on rejection",
        );
    }
    Ok(())
}

pub(super) fn orthographic_projection(
    world: &World,
    camera: Entity,
) -> Result<&OrthographicProjection, &'static str> {
    match world.get::<Projection>(camera) {
        Some(Projection::Orthographic(projection)) => Ok(projection),
        _ => Err("the camera must retain its orthographic projection"),
    }
}

pub(super) fn assert_approximately_equal(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= f32::EPSILON,
        "expected {actual} to approximately equal {expected}",
    );
}

pub(super) fn assert_selected_direct_replacement_state(
    app: &App,
    camera: Entity,
    selected: Entity,
    expected_start: CameraPose,
) -> TestResult {
    let replaced = app
        .world()
        .get::<CameraSequencePlayback>(camera)
        .ok_or("the selected replacement is prepared")?;
    assert_eq!(replaced.start.pose(), expected_start);
    assert!(matches!(
        replaced.lifecycle,
        CameraPlaybackLifecycleState::Effective {
            owner: SequenceOwner::Driver(driver),
        } if driver == selected
    ));
    assert_eq!(
        app.world().resource::<LifecycleEventOrder>().0,
        Vec::<&str>::new()
    );
    assert!(
        app.world()
            .resource::<CameraBoundaryEventOrder>()
            .0
            .is_empty()
    );
    let transitions = app.world().resource::<ControllerOverrideTransitionCounts>();
    assert_eq!((transitions.captured, transitions.released), (1, 0));
    let stash = app
        .world()
        .get::<OrbitControllerOverrideRestoration>(camera)
        .ok_or("replacement keeps the original override continuously")?;
    assert_eq!((stash.zoom, stash.pan, stash.orbit), (0.2, 0.3, 0.4));
    Ok(())
}

pub(super) fn release_stale_selected_driver(
    app: &mut App,
    camera: Entity,
    selected: Entity,
    displaced: Entity,
) -> TestResult {
    app.world_mut()
        .get_mut::<SequenceSourceState>(displaced)
        .ok_or("the displaced driver retains source state")?
        .mark_current();
    app.world_mut()
        .entity_mut(selected)
        .remove::<SequenceDriver>();
    app.update();

    assert_eq!(
        app.world().resource::<ObservedDriverRestorations>().0,
        [bevy_kana::DriverRestoration::DisplacedDriverStale(
            displaced,
        )]
    );
    assert_eq!(
        app.world().resource::<LifecycleEventOrder>().0,
        Vec::<&str>::new()
    );
    let transitions = app.world().resource::<ControllerOverrideTransitionCounts>();
    assert_eq!((transitions.captured, transitions.released), (1, 1));
    assert!(
        app.world()
            .get::<OrbitControllerOverrideRestoration>(camera)
            .is_none()
    );
    assert_eq!(
        app.world_mut()
            .run_system_cached_with(observe_camera_commands, camera)
            .map_err(|_| "post-release camera observation did not run")?
            .0,
        SequenceOwnership::Retained(SequenceOwner::NativePlayback)
    );
    Ok(())
}

pub(super) fn assert_restored_takeover_state(
    app: &App,
    camera: Entity,
    displaced: Entity,
    expected_pose: OrbitCameraPose,
) -> TestResult {
    assert_eq!(
        app.world().resource::<ObservedDriverRestorations>().0,
        [bevy_kana::DriverRestoration::Restored(displaced)]
    );
    assert_eq!(
        app.world().resource::<LifecycleEventOrder>().0,
        Vec::<&str>::new()
    );
    assert!(
        app.world()
            .resource::<CameraBoundaryEventOrder>()
            .0
            .is_empty()
    );
    let transitions = app.world().resource::<ControllerOverrideTransitionCounts>();
    assert_eq!((transitions.captured, transitions.released), (1, 0));
    let orbit = app
        .world()
        .get::<OrbitCam>(camera)
        .ok_or("the restored selected camera retains its controller")?;
    assert_eq!(
        (
            orbit.pan.current(),
            orbit.pan.target(),
            orbit.orbit.current(),
            orbit.orbit.target(),
            orbit.zoom.current(),
            orbit.zoom.target(),
        ),
        (
            expected_pose.focus,
            expected_pose.focus,
            expected_pose.orbit_angles,
            expected_pose.orbit_angles,
            expected_pose.radius,
            expected_pose.radius,
        )
    );
    Ok(())
}

pub(super) fn release_restored_driver_and_assert_idle(
    app: &mut App,
    camera: Entity,
    displaced: Entity,
) -> TestResult {
    app.world_mut()
        .entity_mut(displaced)
        .remove::<SequenceDriver>();
    app.update();

    assert_eq!(
        app.world().resource::<ObservedDriverRestorations>().0,
        [
            bevy_kana::DriverRestoration::Restored(displaced),
            bevy_kana::DriverRestoration::NoDisplacedDriver,
        ]
    );
    assert_eq!(
        app.world().resource::<LifecycleEventOrder>().0,
        Vec::<&str>::new()
    );
    let transitions = app.world().resource::<ControllerOverrideTransitionCounts>();
    assert_eq!((transitions.captured, transitions.released), (1, 1));
    assert!(
        app.world()
            .get::<OrbitControllerOverrideRestoration>(camera)
            .is_none()
    );

    let transform_before = *app
        .world()
        .get::<Transform>(camera)
        .ok_or("the idle camera retains its transform")?;
    let transform_tick_before = app
        .world()
        .entity(camera)
        .get_ref::<Transform>()
        .ok_or("the idle camera exposes its transform change tick")?
        .last_changed();

    app.update();

    assert_eq!(
        app.world()
            .get::<Transform>(camera)
            .ok_or("the idle camera retains its transform")?,
        &transform_before
    );
    assert_eq!(
        app.world()
            .entity(camera)
            .get_ref::<Transform>()
            .ok_or("the idle camera exposes its transform change tick")?
            .last_changed(),
        transform_tick_before
    );
    assert_eq!(
        app.world().resource::<LifecycleEventOrder>().0,
        Vec::<&str>::new()
    );
    assert!(
        app.world()
            .resource::<CameraBoundaryEventOrder>()
            .0
            .is_empty()
    );
    Ok(())
}

pub(super) fn assert_orbit_replacement_state(
    app: &App,
    camera: Entity,
    expected_start: CameraPose,
) -> TestResult {
    let CameraPose::Orbit(expected_pose) = expected_start else {
        return Err("the replacement start must be an orbit pose");
    };
    let retained = app
        .world()
        .get::<CameraSequencePlayback>(camera)
        .ok_or("the replacement controller is prepared")?;
    assert_eq!(retained.start.pose(), expected_start);
    assert_eq!(retained.playback.position(), SequencePosition::START);
    assert_eq!(
        app.world().resource::<LifecycleEventOrder>().0,
        Vec::<&str>::new()
    );
    assert!(
        app.world()
            .resource::<CameraBoundaryEventOrder>()
            .0
            .is_empty()
    );
    let stash = app
        .world()
        .get::<OrbitControllerOverrideRestoration>(camera)
        .ok_or("the replacement controller owns a new override stash")?;
    assert_eq!((stash.zoom, stash.pan, stash.orbit), (0.6, 0.7, 0.8));
    let orbit = app
        .world()
        .get::<OrbitCam>(camera)
        .ok_or("the replacement orbit controller remains installed")?;
    assert_eq!(
        (
            orbit.pan.current(),
            orbit.pan.target(),
            orbit.orbit.current(),
            orbit.orbit.target(),
            orbit.zoom.current(),
            orbit.zoom.target(),
        ),
        (
            expected_pose.focus,
            expected_pose.focus,
            expected_pose.orbit_angles,
            expected_pose.orbit_angles,
            expected_pose.radius,
            expected_pose.radius,
        )
    );
    assert_eq!(
        (
            orbit.zoom.damping(),
            orbit.pan.damping(),
            orbit.orbit.damping(),
        ),
        (0.0, 0.0, 0.0)
    );
    let transitions = app.world().resource::<ControllerOverrideTransitionCounts>();
    assert_eq!((transitions.captured, transitions.released), (2, 1));
    Ok(())
}

pub(super) fn release_orbit_replacement_override(
    app: &mut App,
    camera: Entity,
    driver: Entity,
) -> TestResult {
    app.world_mut()
        .entity_mut(driver)
        .remove::<SequenceDriver>();
    app.update();

    let orbit = app
        .world()
        .get::<OrbitCam>(camera)
        .ok_or("the released replacement orbit controller remains installed")?;
    assert_eq!(
        (
            orbit.zoom.damping(),
            orbit.pan.damping(),
            orbit.orbit.damping(),
        ),
        (0.6, 0.7, 0.8)
    );
    let transitions = app.world().resource::<ControllerOverrideTransitionCounts>();
    assert_eq!((transitions.captured, transitions.released), (2, 2));
    assert!(
        app.world()
            .get::<OrbitControllerOverrideRestoration>(camera)
            .is_none()
    );
    assert_eq!(
        app.world().resource::<LifecycleEventOrder>().0,
        Vec::<&str>::new()
    );
    Ok(())
}

pub(super) fn assert_cross_kind_replacement_state(
    app: &App,
    camera: Entity,
    expected_start: CameraPose,
) -> TestResult {
    let CameraPose::Free(expected_pose) = expected_start else {
        return Err("the cross-kind replacement start must be a free-flight pose");
    };
    let retained = app
        .world()
        .get::<CameraSequencePlayback>(camera)
        .ok_or("the cross-kind replacement is prepared")?;
    assert_eq!(retained.start.pose(), expected_start);
    assert_eq!(retained.playback.position(), SequencePosition::START);
    assert_eq!(
        app.world().resource::<LifecycleEventOrder>().0,
        Vec::<&str>::new()
    );
    assert!(
        app.world()
            .resource::<CameraBoundaryEventOrder>()
            .0
            .is_empty()
    );
    assert!(
        app.world()
            .get::<OrbitControllerOverrideRestoration>(camera)
            .is_none()
    );
    let stash = app
        .world()
        .get::<FreeFlightControllerOverrideRestoration>(camera)
        .ok_or("the free-flight replacement owns a new override stash")?;
    assert_eq!((stash.translate, stash.look, stash.roll), (0.6, 0.7, 0.8));
    let free = app
        .world()
        .get::<FreeCam>(camera)
        .ok_or("the replacement free-flight controller remains installed")?;
    assert_eq!(
        (
            free.translate.current(),
            free.translate.target(),
            free.look.current(),
            free.look.target(),
            free.roll.current(),
            free.roll.target(),
        ),
        (
            expected_pose.position,
            expected_pose.position,
            expected_pose.look,
            expected_pose.look,
            expected_pose.roll,
            expected_pose.roll,
        )
    );
    assert_eq!(
        (
            free.translate.damping(),
            free.look.damping(),
            free.roll.damping(),
        ),
        (0.0, 0.0, 0.0)
    );
    let transitions = app.world().resource::<ControllerOverrideTransitionCounts>();
    assert_eq!((transitions.captured, transitions.released), (2, 1));
    Ok(())
}

pub(super) fn release_cross_kind_replacement_override(
    app: &mut App,
    camera: Entity,
    driver: Entity,
) -> TestResult {
    app.world_mut()
        .entity_mut(driver)
        .remove::<SequenceDriver>();
    app.update();

    let free = app
        .world()
        .get::<FreeCam>(camera)
        .ok_or("the released free-flight controller remains installed")?;
    assert_eq!(
        (
            free.translate.damping(),
            free.look.damping(),
            free.roll.damping(),
        ),
        (0.6, 0.7, 0.8)
    );
    let transitions = app.world().resource::<ControllerOverrideTransitionCounts>();
    assert_eq!((transitions.captured, transitions.released), (2, 2));
    assert!(
        app.world()
            .get::<FreeFlightControllerOverrideRestoration>(camera)
            .is_none()
    );
    assert_eq!(
        app.world().resource::<LifecycleEventOrder>().0,
        Vec::<&str>::new()
    );
    Ok(())
}

pub(super) fn assert_reflected_camera_event<T>(
    registry: &TypeRegistry,
    expected_type_path: &str,
) -> TestResult
where
    T: TypePath + 'static,
{
    let registration = registry
        .get(TypeId::of::<T>())
        .ok_or("a public camera entity event was not automatically registered")?;
    assert_eq!(registration.type_info().type_path(), expected_type_path);
    assert!(
        registration.data::<ReflectEvent>().is_some(),
        "{expected_type_path} must retain Bevy reflected event data",
    );
    assert!(
        registration.data::<ReflectFromReflect>().is_some(),
        "{expected_type_path} must retain Bevy reflected from-reflect data",
    );
    Ok(())
}

pub(super) fn assert_reflected_camera_payload<T>(
    registry: &TypeRegistry,
    expected_type_path: &str,
) -> TestResult
where
    T: TypePath + 'static,
{
    let registration = registry
        .get(TypeId::of::<T>())
        .ok_or("a public camera event payload was not automatically registered")?;
    assert_eq!(registration.type_info().type_path(), expected_type_path);
    assert!(
        registration.data::<ReflectFromReflect>().is_some(),
        "{expected_type_path} must retain Bevy reflected from-reflect data",
    );
    Ok(())
}

pub(super) fn forward_three_move_boundaries(
    camera: Entity,
    owner: SequenceOwner,
    stages: [(SequenceStageId, SequenceStageSpan); 3],
    timings: [CameraEventTiming; 3],
) -> [CameraEventTraceItem; 6] {
    let [first_stage, second_stage, third_stage] = stages;
    let [start_timing, middle_timing, end_timing] = timings;
    [
        CameraEventTraceItem::CameraMoveBegin {
            camera,
            stage_id: first_stage.0,
            owner,
            direction: SequenceDirection::Forward,
            boundary_elapsed: first_stage.1.start(),
            timing: start_timing,
        },
        CameraEventTraceItem::CameraMoveEnd {
            camera,
            stage_id: first_stage.0,
            owner,
            direction: SequenceDirection::Forward,
            boundary_elapsed: first_stage.1.end(),
            timing: middle_timing,
        },
        CameraEventTraceItem::CameraMoveBegin {
            camera,
            stage_id: second_stage.0,
            owner,
            direction: SequenceDirection::Forward,
            boundary_elapsed: second_stage.1.start(),
            timing: middle_timing,
        },
        CameraEventTraceItem::CameraMoveEnd {
            camera,
            stage_id: second_stage.0,
            owner,
            direction: SequenceDirection::Forward,
            boundary_elapsed: second_stage.1.end(),
            timing: middle_timing,
        },
        CameraEventTraceItem::CameraMoveBegin {
            camera,
            stage_id: third_stage.0,
            owner,
            direction: SequenceDirection::Forward,
            boundary_elapsed: third_stage.1.start(),
            timing: middle_timing,
        },
        CameraEventTraceItem::CameraMoveEnd {
            camera,
            stage_id: third_stage.0,
            owner,
            direction: SequenceDirection::Forward,
            boundary_elapsed: third_stage.1.end(),
            timing: end_timing,
        },
    ]
}

pub(super) fn spawn_zero_duration_facade_target(app: &mut App, position: Vec3) -> Entity {
    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.0, 1.0, 1.0));
    app.world_mut()
        .spawn((
            Mesh3d(mesh),
            Transform::from_translation(position),
            GlobalTransform::from(Transform::from_translation(position)),
        ))
        .id()
}

pub(super) fn spawn_zero_duration_free_camera(app: &mut App, position: Vec3) -> Entity {
    app.world_mut()
        .spawn((
            free_camera(position, 0.25, -0.1, 0.35),
            CameraBasis::Y_UP,
            Camera::default(),
            Projection::Perspective(PerspectiveProjection::default()),
            Transform::from_translation(position),
            GlobalTransform::from(Transform::from_translation(position)),
        ))
        .id()
}

#[derive(Clone, Copy)]
pub(super) enum ZeroDurationFacadeTraceShape {
    ZoomWithOneAuthoredMove,
    OneAuthoredMove,
    TwoAuthoredMoves,
}

pub(super) fn zero_duration_animation_begin(
    camera: Entity,
    target: Entity,
    source: AnimationSource,
    timing: CameraEventTiming,
) -> CameraEventTraceItem {
    CameraEventTraceItem::AnimationBegin {
        camera,
        source,
        target: Some(target),
        owner: SequenceOwner::NativePlayback,
        direction: SequenceDirection::Forward,
        timing,
    }
}

pub(super) fn zero_duration_move_pair(
    camera: Entity,
    stage_id: SequenceStageId,
    timing: CameraEventTiming,
) -> [CameraEventTraceItem; 2] {
    [
        CameraEventTraceItem::CameraMoveBegin {
            camera,
            stage_id,
            owner: SequenceOwner::NativePlayback,
            direction: SequenceDirection::Forward,
            boundary_elapsed: SequenceTime::ZERO,
            timing,
        },
        CameraEventTraceItem::CameraMoveEnd {
            camera,
            stage_id,
            owner: SequenceOwner::NativePlayback,
            direction: SequenceDirection::Forward,
            boundary_elapsed: SequenceTime::ZERO,
            timing,
        },
    ]
}

pub(super) fn zero_duration_animation_end(
    camera: Entity,
    target: Entity,
    source: AnimationSource,
    timing: CameraEventTiming,
) -> CameraEventTraceItem {
    CameraEventTraceItem::AnimationEnd {
        camera,
        source,
        target: Some(target),
        owner: SequenceOwner::NativePlayback,
        direction: SequenceDirection::Forward,
        timing,
        outcome: CameraEventOutcome::Completed,
    }
}

pub(super) fn zero_duration_one_move_trace(
    camera: Entity,
    target: Entity,
    source: AnimationSource,
    stage_id: SequenceStageId,
    start_timing: CameraEventTiming,
    end_timing: CameraEventTiming,
) -> Vec<CameraEventTraceItem> {
    let mut expected = vec![zero_duration_animation_begin(
        camera,
        target,
        source,
        start_timing,
    )];
    expected.extend(zero_duration_move_pair(camera, stage_id, start_timing));
    expected.push(zero_duration_animation_end(
        camera, target, source, end_timing,
    ));
    expected
}

pub(super) fn zero_duration_zoom_trace(
    camera: Entity,
    target: Entity,
    source: AnimationSource,
    stage_id: SequenceStageId,
    start_timing: CameraEventTiming,
    end_timing: CameraEventTiming,
) -> Vec<CameraEventTraceItem> {
    let mut expected = vec![CameraEventTraceItem::ZoomBegin { camera, target }];
    expected.extend(zero_duration_one_move_trace(
        camera,
        target,
        source,
        stage_id,
        start_timing,
        end_timing,
    ));
    expected.push(CameraEventTraceItem::ZoomEnd {
        camera,
        target,
        outcome: CameraEventOutcome::Completed,
    });
    expected
}

pub(super) fn zero_duration_two_move_trace(
    camera: Entity,
    target: Entity,
    source: AnimationSource,
    stage_ids: [SequenceStageId; 2],
    start_timing: CameraEventTiming,
    end_timing: CameraEventTiming,
) -> Vec<CameraEventTraceItem> {
    let mut expected = vec![zero_duration_animation_begin(
        camera,
        target,
        source,
        start_timing,
    )];
    for stage_id in stage_ids {
        expected.extend(zero_duration_move_pair(camera, stage_id, start_timing));
    }
    expected.push(zero_duration_animation_end(
        camera, target, source, end_timing,
    ));
    expected
}

pub(super) fn assert_zero_duration_native_facade_trace(
    app: &App,
    camera: Entity,
    target: Entity,
    source: AnimationSource,
    trace_shape: ZeroDurationFacadeTraceShape,
) -> TestResult {
    let sequence = app
        .world()
        .get::<CameraSequence>(camera)
        .ok_or("the accepted public facade must retain its sequence")?;
    let authored_stage_ids: Vec<_> = sequence
        .stage_ids_with_spans()
        .map(|(stage_id, _)| stage_id)
        .collect();
    let start_timing = CameraEventTiming::new(SequencePosition::START, sequence.total());
    let end_timing = CameraEventTiming::new(SequencePosition::END, sequence.total());
    let expected = match trace_shape {
        ZeroDurationFacadeTraceShape::ZoomWithOneAuthoredMove => {
            let [stage_id] = authored_stage_ids.as_slice() else {
                return Err("ZoomToFit must retain exactly one authored stage");
            };
            zero_duration_zoom_trace(camera, target, source, *stage_id, start_timing, end_timing)
        },
        ZeroDurationFacadeTraceShape::OneAuthoredMove => {
            let [stage_id] = authored_stage_ids.as_slice() else {
                return Err("the facade must retain exactly one authored stage");
            };
            zero_duration_one_move_trace(
                camera,
                target,
                source,
                *stage_id,
                start_timing,
                end_timing,
            )
        },
        ZeroDurationFacadeTraceShape::TwoAuthoredMoves => {
            let [first_stage_id, second_stage_id] = authored_stage_ids.as_slice() else {
                return Err("the two-move facade must retain exactly two authored stages");
            };
            assert_ne!(first_stage_id, second_stage_id);
            zero_duration_two_move_trace(
                camera,
                target,
                source,
                [*first_stage_id, *second_stage_id],
                start_timing,
                end_timing,
            )
        },
    };
    assert_eq!(
        app.world().resource::<CameraEventTrace>().0,
        expected,
        "the zero-duration facade must preserve its exact public event order",
    );
    Ok(())
}

pub(super) fn assert_facade_journey_metadata(
    world: &World,
    camera: Entity,
    revision: SequenceStagesRevision,
    target: Entity,
) -> TestResult {
    let journey = world
        .get::<RetainedCameraJourney>(camera)
        .ok_or("the revision-bound facade journey is retained")?;
    assert_eq!(journey.revision, revision);
    assert!(matches!(
        journey.origin,
        RetainedCameraJourneyOrigin::Facade(AnimationSource::ZoomToFit)
    ));
    assert_eq!(journey.target, Some(target));
    let zoom = journey
        .zoom
        .as_ref()
        .ok_or("the facade journey retains its zoom context")?;
    assert_eq!(zoom.target, target);
    assert_approximately_equal(zoom.margin, 0.27);
    assert_eq!(zoom.duration, Duration::from_secs(3));
    assert_eq!(zoom.easing, EaseFunction::SineInOut);
    Ok(())
}

pub(super) fn orbit_camera(focus: Vec3, yaw: f32, pitch: f32, radius: f32) -> OrbitCam {
    OrbitCam::from_pose(Focus(focus), OrbitAngles { yaw, pitch }, Radius(radius))
}

pub(super) fn free_camera(position: Vec3, yaw: f32, pitch: f32, roll: f32) -> FreeCam {
    FreeCam::from_pose(Position(position), LookAngles { yaw, pitch }, Roll(roll))
}

pub(super) fn orbital_move(
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    radius: f32,
    duration: Duration,
) -> CameraMove {
    CameraMove::try_to_orbital_look_at(
        Focus(focus),
        OrbitAngles { yaw, pitch },
        Radius(radius),
        FreeCamRollTarget::InheritPrevious,
        duration,
        EaseFunction::Linear,
    )
    .expect("the authored orbital test pose is finite")
}

pub(super) fn free_move(
    position: Vec3,
    focus: Vec3,
    roll: FreeCamRollTarget,
    duration: Duration,
) -> CameraMove {
    CameraMove::try_to_look_at(
        Position(position),
        Focus(focus),
        roll,
        duration,
        EaseFunction::Linear,
    )
    .expect("the authored free-flight test pose is finite")
}

pub(super) fn prepared_orbit(sequence: &CameraSequence) -> CameraSequencePlayback {
    CameraSequencePlayback::prepare_for_orbit(
        sequence,
        &orbit_camera(Vec3::ZERO, 0.0, 0.0, 1.0),
        OrbitControllerInstallation::default(),
    )
    .expect("the authored orbit sequence prepares")
}

pub(super) fn moving_sample(
    playback: &CameraSequencePlayback,
    position: f32,
) -> Result<CameraMoveSample<'_>, &'static str> {
    let position =
        SequencePosition::try_new(position).map_err(|_| "the test position is normalized")?;
    let sample = playback.sample(position);
    if matches!(sample, CameraMoveSample::Moving { .. }) {
        Ok(sample)
    } else {
        Err("the test position is inside a positive-duration move")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RecordedPreparationRejection {
    pub(super) camera: Entity,
    pub(super) source: AnimationSource,
    pub(super) target: Entity,
    pub(super) error:  CameraRequestPreparationError,
}

#[derive(Resource, Default)]
pub(super) struct HigherRequestEventRecord {
    pub(super) rejections:      Vec<RecordedPreparationRejection>,
    pub(super) animation_begin: usize,
    pub(super) animation_end:   usize,
    pub(super) zoom_begin:      usize,
    pub(super) zoom_end:        usize,
}

#[derive(Clone, Debug, Default)]
pub(super) struct UnsupportedTestProjection(pub(super) PerspectiveProjection);

impl CameraProjection for UnsupportedTestProjection {
    fn get_clip_from_view(&self) -> Mat4 { self.0.get_clip_from_view() }

    fn get_clip_from_view_for_sub(&self, sub_view: &SubCameraView) -> Mat4 {
        self.0.get_clip_from_view_for_sub(sub_view)
    }

    fn update(&mut self, width: f32, height: f32) { self.0.update(width, height); }

    fn far(&self) -> f32 { self.0.far }

    fn get_frustum_corners(&self, z_near: f32, z_far: f32) -> [Vec3A; 8] {
        self.0.get_frustum_corners(z_near, z_far)
    }
}

pub(super) fn higher_request_test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AnimationPlugin, FitPlugin))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<HigherRequestEventRecord>()
        .add_observer(record_preparation_rejection)
        .add_observer(record_animation_begin)
        .add_observer(record_animation_end)
        .add_observer(record_zoom_begin)
        .add_observer(record_zoom_end);
    app.finish();
    app
}

pub(super) fn record_preparation_rejection(
    rejected: On<AnimationRejected>,
    mut record: ResMut<HigherRequestEventRecord>,
) {
    let AnimationRejectionReason::RequestPreparationFailed(error) = &rejected.reason else {
        panic!("expected a typed higher-request preparation rejection");
    };
    let Some(target) = rejected.target else {
        panic!("a higher-request rejection must preserve its target");
    };
    record.rejections.push(RecordedPreparationRejection {
        camera: rejected.camera,
        source: rejected.source,
        target,
        error: *error,
    });
}

pub(super) fn record_animation_begin(
    _: On<AnimationBegin>,
    mut record: ResMut<HigherRequestEventRecord>,
) {
    record.animation_begin += 1;
}

pub(super) fn record_animation_end(
    _: On<AnimationEnd>,
    mut record: ResMut<HigherRequestEventRecord>,
) {
    record.animation_end += 1;
}

pub(super) fn record_zoom_begin(_: On<ZoomBegin>, mut record: ResMut<HigherRequestEventRecord>) {
    record.zoom_begin += 1;
}

pub(super) fn record_zoom_end(_: On<ZoomEnd>, mut record: ResMut<HigherRequestEventRecord>) {
    record.zoom_end += 1;
}

pub(super) fn spawn_orbit_camera(app: &mut App) -> Entity {
    app.world_mut()
        .spawn((
            OrbitCam::from_pose(
                Focus(Vec3::ZERO),
                OrbitAngles {
                    yaw:   0.25,
                    pitch: 0.5,
                },
                Radius(8.0),
            ),
            Camera::default(),
            Projection::Perspective(PerspectiveProjection::default()),
            Transform::from_xyz(0.0, 0.0, 8.0),
            GlobalTransform::from(Transform::from_xyz(0.0, 0.0, 8.0)),
        ))
        .id()
}

pub(super) fn spawn_target_with_mesh(app: &mut App, mesh: Mesh) -> Entity {
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    app.world_mut()
        .spawn((
            Mesh3d(mesh),
            Transform::default(),
            GlobalTransform::default(),
        ))
        .id()
}

pub(super) fn spawn_target(app: &mut App) -> Entity {
    spawn_target_with_mesh(app, Cuboid::new(1.0, 1.0, 1.0).into())
}

pub(super) fn camera_observation(
    In(camera): In<Entity>,
    commands: CameraCommands,
) -> (SequenceOwnership, CameraPlaybackObservation) {
    (commands.owner(camera), commands.observe(camera))
}

pub(super) fn assert_typed_rejection_without_mutation(
    app: &mut App,
    camera: Entity,
    original_controller: OrbitCam,
    source: AnimationSource,
    target: Entity,
    error: CameraRequestPreparationError,
) {
    let record = app.world().resource::<HigherRequestEventRecord>();
    assert_eq!(
        record.rejections,
        [RecordedPreparationRejection {
            camera,
            source,
            target,
            error,
        }]
    );
    assert_eq!(
        (
            record.animation_begin,
            record.animation_end,
            record.zoom_begin,
            record.zoom_end,
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(
        app.world().get::<OrbitCam>(camera),
        Some(&original_controller)
    );
    assert!(app.world().get::<CameraSequence>(camera).is_none());
    assert!(app.world().get::<SequenceStages>(camera).is_none());
    assert!(app.world().get::<CameraSequencePlayback>(camera).is_none());
    assert!(app.world().get::<RetainedCameraJourney>(camera).is_none());
    assert!(app.world().get::<NativeCameraPlayRequest>(camera).is_none());
    assert!(
        app.world()
            .get::<CameraSequencePreparationRequested>(camera)
            .is_none()
    );
    assert!(app.world().get::<CurrentFitTarget>(camera).is_none());
    assert!(
        app.world()
            .get::<OrbitControllerOverrideRestoration>(camera)
            .is_none()
    );
    assert!(
        app.world()
            .get::<FreeFlightControllerOverrideRestoration>(camera)
            .is_none()
    );
    assert!(app.world().resource::<PendingCameraRequests>().0.is_empty());
    assert_eq!(
        app.world_mut()
            .run_system_cached_with(camera_observation, camera)
            .expect("camera observation should run"),
        (
            SequenceOwnership::NoRetainedSequence,
            CameraPlaybackObservation::NoRetainedSequence,
        )
    );
    match error {
        CameraRequestPreparationError::MissingProjection => {
            assert!(app.world().get::<Projection>(camera).is_none());
        },
        CameraRequestPreparationError::UnsupportedProjection => {
            assert!(matches!(
                app.world().get::<Projection>(camera),
                Some(Projection::Custom(_))
            ));
        },
        _ => {
            assert!(matches!(
                app.world().get::<Projection>(camera),
                Some(Projection::Perspective(_))
            ));
        },
    }
}
