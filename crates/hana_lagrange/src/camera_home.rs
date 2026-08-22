//! Shared home-pose capture and lock behavior for camera kinds.

use bevy::ecs::event::EntityEvent;
use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::With;

use crate::FreeCam;
use crate::FreeCamHomePose;
use crate::FreeCamKind;
use crate::OrbitCam;
use crate::OrbitCamHomePose;
use crate::OrbitCamKind;
use crate::animation::AnimationEnd;
use crate::animation::AnimationReason;
use crate::input::CameraHomed;
use crate::input::CameraInputKind;
use crate::input::InteractionSources;
use crate::input::ResetFreeCamToHome;
use crate::input::ResetOrbitCamToHome;

/// A camera kind that supports a captured home/reset pose.
pub trait CameraHomeKind: CameraInputKind {
    /// The per-kind stored home pose.
    type HomePose: Component + Copy + Reflect;
    /// The rising-edge interaction event for this kind.
    type InteractionStarted: EntityEvent;

    /// Snapshot the current settled pose as a home pose.
    fn capture_home(camera: &Self::Camera) -> Self::HomePose;

    /// Re-target the camera's eased operations toward `home`.
    fn apply_home(camera: &mut Self::Camera, home: &Self::HomePose);
}

/// Marks a camera whose captured home pose is still provisional.
///
/// While present, a completed [`AnimationEnd`] replaces the stored home pose with
/// the settled pose and removes this marker, and the kind's interaction-started
/// event removes it without recapturing. Absent it, the stored home pose is fixed.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct CameraHomePending;

#[derive(Component)]
pub(crate) struct CameraHomeResetSources(pub(crate) InteractionSources);

pub(crate) fn add_home_systems<K: CameraHomeKind>(app: &mut App) {
    app.add_observer(on_animation_settled::<K>)
        .add_observer(on_interaction_locks_home::<K>);
}

pub(crate) fn add_orbit_cam_home_reset_systems(app: &mut App) {
    app.add_observer(on_reset_orbit_cam_to_home);
}

pub(crate) fn add_free_cam_home_reset_systems(app: &mut App) {
    app.add_observer(on_reset_free_cam_to_home);
}

fn on_reset_orbit_cam_to_home(
    event: On<ResetOrbitCamToHome>,
    mut cameras: Query<(
        &mut OrbitCam,
        &OrbitCamHomePose,
        Option<&CameraHomeResetSources>,
    )>,
    mut commands: Commands,
) {
    let camera = event.camera;
    if let Ok((mut orbit_cam, home, reset_sources)) = cameras.get_mut(camera) {
        OrbitCamKind::apply_home(&mut orbit_cam, home);
        commands.trigger(CameraHomed {
            camera,
            sources: reset_sources.map_or(InteractionSources::NONE, |sources| sources.0),
        });
        commands.entity(camera).remove::<CameraHomeResetSources>();
    }
}

fn on_reset_free_cam_to_home(
    event: On<ResetFreeCamToHome>,
    mut cameras: Query<(
        &mut FreeCam,
        &FreeCamHomePose,
        Option<&CameraHomeResetSources>,
    )>,
    mut commands: Commands,
) {
    let camera = event.camera;
    if let Ok((mut free_cam, home, reset_sources)) = cameras.get_mut(camera) {
        FreeCamKind::apply_home(&mut free_cam, home);
        commands.trigger(CameraHomed {
            camera,
            sources: reset_sources.map_or(InteractionSources::NONE, |sources| sources.0),
        });
        commands.entity(camera).remove::<CameraHomeResetSources>();
    }
}

fn on_animation_settled<K: CameraHomeKind>(
    event: On<AnimationEnd>,
    cameras: Query<&K::Camera, With<CameraHomePending>>,
    mut commands: Commands,
) {
    if matches!(&event.reason, AnimationReason::Cancelled { .. }) {
        return;
    }

    if let Ok(camera) = cameras.get(event.camera) {
        commands
            .entity(event.camera)
            .insert(K::capture_home(camera))
            .remove::<CameraHomePending>();
    }
}

fn on_interaction_locks_home<K: CameraHomeKind>(
    event: On<K::InteractionStarted>,
    pending: Query<(), With<CameraHomePending>>,
    mut commands: Commands,
) {
    let camera = event.event_target();
    if pending.get(camera).is_ok() {
        commands.entity(camera).remove::<CameraHomePending>();
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use bevy::camera::RenderTarget;
    use bevy::input::mouse::AccumulatedMouseMotion;
    use bevy::input::mouse::AccumulatedMouseScroll;
    use bevy::math::curve::easing::EaseFunction;
    use bevy::prelude::App;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::Camera;
    use bevy::prelude::Entity;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::MouseButton;
    use bevy::prelude::On;
    use bevy::prelude::PreUpdate;
    use bevy::prelude::Query;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec3;
    use bevy::prelude::With;
    use bevy::window::WindowRef;

    use super::CameraHomePending;
    use crate::AnimationEnd;
    use crate::AnimationReason;
    use crate::AnimationSource;
    use crate::CameraBasis;
    use crate::CameraEventTiming;
    use crate::CameraHomed;
    use crate::CameraInputPhase;
    use crate::CameraMove;
    use crate::Focus;
    use crate::FreeCam;
    use crate::FreeCamHomePose;
    use crate::FreeCamInput;
    use crate::FreeCamInputMode;
    use crate::FreeCamInteractionKind;
    use crate::FreeCamInteractionStarted;
    use crate::FreeCamManualInputWriter;
    use crate::FreeCamRollTarget;
    use crate::Initialization;
    use crate::InteractionSources;
    use crate::LagrangePlugin;
    use crate::LookAngles;
    use crate::ManualInputSource;
    use crate::OrbitAngles;
    use crate::OrbitCam;
    use crate::OrbitCamHomePose;
    use crate::Position;
    use crate::Radius;
    use crate::ResetFreeCamToHome;
    use crate::ResetOrbitCamToHome;
    use crate::Roll;
    use crate::SequenceDirection;
    use crate::SequenceOwner;
    use crate::SequencePosition;
    use crate::SequenceStageId;
    use crate::SequenceStages;
    use crate::SequenceTime;
    use crate::TimeSource;

    const START_POSITION: Vec3 = Vec3::new(1.0, 2.0, 3.0);
    const START_YAW: f32 = 0.25;
    const START_PITCH: f32 = -0.125;
    const START_ROLL: f32 = 0.5;

    const APP_HOME_POSITION: Vec3 = Vec3::new(4.0, 5.0, 6.0);
    const APP_HOME_YAW: f32 = 0.75;
    const APP_HOME_PITCH: f32 = 0.125;
    const APP_HOME_ROLL: f32 = -0.25;

    const SETTLED_POSITION: Vec3 = Vec3::new(-2.0, 1.0, 8.0);
    const SETTLED_YAW: f32 = 1.25;
    const SETTLED_PITCH: f32 = 0.375;
    const SETTLED_ROLL: f32 = 0.875;

    const INTERRUPTED_MOVE_DURATION: Duration = Duration::from_millis(250);
    const ORBIT_HOME_FOCUS: Vec3 = Vec3::new(10.0, 11.0, 12.0);
    const ORBIT_HOME_YAW: f32 = 0.75;
    const ORBIT_HOME_PITCH: f32 = -0.25;
    const ORBIT_HOME_RADIUS: f32 = 6.0;
    const ORBIT_AWAY_FOCUS: Vec3 = Vec3::new(-1.0, -2.0, -3.0);
    const ORBIT_AWAY_YAW: f32 = 1.5;
    const ORBIT_AWAY_PITCH: f32 = 0.5;
    const ORBIT_AWAY_RADIUS: f32 = 12.0;

    type TestResult = Result<(), &'static str>;

    fn home_pose(position: Vec3, yaw: f32, pitch: f32, roll: f32) -> FreeCamHomePose {
        FreeCamHomePose {
            position: Position(position),
            look:     LookAngles { yaw, pitch },
            roll:     Roll(roll),
        }
    }

    fn start_pose() -> FreeCamHomePose {
        home_pose(START_POSITION, START_YAW, START_PITCH, START_ROLL)
    }

    fn app_home_pose() -> FreeCamHomePose {
        home_pose(
            APP_HOME_POSITION,
            APP_HOME_YAW,
            APP_HOME_PITCH,
            APP_HOME_ROLL,
        )
    }

    fn settled_pose() -> FreeCamHomePose {
        home_pose(SETTLED_POSITION, SETTLED_YAW, SETTLED_PITCH, SETTLED_ROLL)
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, LagrangePlugin))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>();
        app.finish();
        app
    }

    fn spawn_free_cam(app: &mut App, home: Option<FreeCamHomePose>) -> Entity {
        let start = start_pose();
        let mut entity = app.world_mut().spawn((
            FreeCam::from_pose(start.position, start.look, start.roll),
            FreeCamInput::default(),
            FreeCamInputMode::Manual,
            Camera::default(),
            RenderTarget::Window(WindowRef::Primary),
            Transform::default(),
            CameraBasis::Y_UP,
            TimeSource::Virtual,
        ));
        if let Some(home) = home {
            entity.insert(home);
        }
        entity.id()
    }

    fn spawn_orbit_cam(app: &mut App, home: OrbitCamHomePose) -> Entity {
        app.world_mut().spawn((OrbitCam::default(), home)).id()
    }

    fn orbit_home_pose() -> OrbitCamHomePose {
        OrbitCamHomePose {
            orbit: OrbitAngles {
                yaw:   ORBIT_HOME_YAW,
                pitch: ORBIT_HOME_PITCH,
            },
            pan:   Focus(ORBIT_HOME_FOCUS),
            zoom:  Radius(ORBIT_HOME_RADIUS),
        }
    }

    fn set_orbit_targets_away(app: &mut App, camera: Entity) -> Result<(), &'static str> {
        let mut orbit_cam = app
            .world_mut()
            .get_mut::<OrbitCam>(camera)
            .ok_or("camera missing OrbitCam")?;
        orbit_cam.orbit.set_target(OrbitAngles {
            yaw:   ORBIT_AWAY_YAW,
            pitch: ORBIT_AWAY_PITCH,
        });
        orbit_cam.pan.set_target(Focus(ORBIT_AWAY_FOCUS));
        orbit_cam.zoom.set_target(Radius(ORBIT_AWAY_RADIUS));
        Ok(())
    }

    fn home_for(app: &App, camera: Entity) -> Result<FreeCamHomePose, &'static str> {
        app.world()
            .get::<FreeCamHomePose>(camera)
            .copied()
            .ok_or("camera missing FreeCamHomePose")
    }

    fn set_current_pose(
        app: &mut App,
        camera: Entity,
        pose: FreeCamHomePose,
    ) -> Result<(), &'static str> {
        let mut free_cam = app
            .world_mut()
            .get_mut::<FreeCam>(camera)
            .ok_or("camera missing FreeCam")?;
        free_cam.translate.snap_to(pose.position);
        free_cam.look.snap_to(pose.look);
        free_cam.roll.snap_to(pose.roll);
        Ok(())
    }

    fn trigger_animation_end(app: &mut App, camera: Entity, reason: AnimationReason) {
        let world = app.world_mut();
        world.trigger(AnimationEnd {
            camera,
            source: AnimationSource::PlayAnimation,
            target: None,
            owner: SequenceOwner::NativePlayback,
            direction: SequenceDirection::Forward,
            timing: CameraEventTiming::new(SequencePosition::START, SequenceTime::ZERO),
            reason,
        });
        world.flush();
    }

    fn interrupted_stage_id() -> SequenceStageId {
        let sequence_stages = SequenceStages::new([Duration::ZERO]);
        sequence_stages
            .stage_ids_with_spans()
            .next()
            .expect("a one-stage fixture always has a stage identity")
            .0
    }

    fn trigger_free_reset(app: &mut App, camera: Entity) {
        let world = app.world_mut();
        world.trigger(ResetFreeCamToHome { camera });
        world.flush();
    }

    fn trigger_orbit_reset(app: &mut App, camera: Entity) {
        let world = app.world_mut();
        world.trigger(ResetOrbitCamToHome { camera });
        world.flush();
    }

    fn interrupted_move() -> CameraMove {
        CameraMove::try_to_look_at(
            Position(Vec3::Z),
            Focus(Vec3::ZERO),
            FreeCamRollTarget::InheritPrevious,
            INTERRUPTED_MOVE_DURATION,
            EaseFunction::Linear,
        )
        .expect("interrupted move is finite")
    }

    #[test]
    fn spawn_without_home_captures_provisional_home() -> TestResult {
        let mut app = test_app();
        let camera = spawn_free_cam(&mut app, None);

        app.update();

        assert_eq!(home_for(&app, camera)?, start_pose());
        assert!(app.world().get::<CameraHomePending>(camera).is_some());
        let free_cam = app
            .world()
            .get::<FreeCam>(camera)
            .ok_or("camera missing FreeCam")?;
        assert_eq!(free_cam.initialization, Initialization::Active);
        Ok(())
    }

    #[test]
    fn app_provided_home_gets_no_pending_marker() -> TestResult {
        let mut app = test_app();
        let camera = spawn_free_cam(&mut app, Some(app_home_pose()));

        app.update();

        assert_eq!(home_for(&app, camera)?, app_home_pose());
        assert!(app.world().get::<CameraHomePending>(camera).is_none());
        Ok(())
    }

    #[test]
    fn completed_animation_upgrades_pending_home() -> TestResult {
        let mut app = test_app();
        let camera = spawn_free_cam(&mut app, None);
        app.update();
        let settled = settled_pose();
        set_current_pose(&mut app, camera, settled)?;

        trigger_animation_end(&mut app, camera, AnimationReason::Completed);

        assert_eq!(home_for(&app, camera)?, settled);
        assert!(app.world().get::<CameraHomePending>(camera).is_none());
        Ok(())
    }

    #[test]
    fn cancelled_animation_leaves_pending_home() -> TestResult {
        let mut app = test_app();
        let camera = spawn_free_cam(&mut app, None);
        app.update();
        set_current_pose(&mut app, camera, settled_pose())?;

        trigger_animation_end(
            &mut app,
            camera,
            AnimationReason::Cancelled {
                interrupted_stage_id: interrupted_stage_id(),
                interrupted_move:     interrupted_move(),
            },
        );

        assert_eq!(home_for(&app, camera)?, start_pose());
        assert!(app.world().get::<CameraHomePending>(camera).is_some());
        Ok(())
    }

    #[test]
    fn first_interaction_locks_pending_home() -> TestResult {
        let mut app = test_app();
        let camera = spawn_free_cam(&mut app, None);
        app.update();
        set_current_pose(&mut app, camera, settled_pose())?;

        let world = app.world_mut();
        world.trigger(FreeCamInteractionStarted {
            camera,
            kind: FreeCamInteractionKind::Translate,
            sources: InteractionSources::KEYBOARD,
        });
        world.flush();

        assert_eq!(home_for(&app, camera)?, start_pose());
        assert!(app.world().get::<CameraHomePending>(camera).is_none());
        Ok(())
    }

    #[test]
    fn target_motion_without_interaction_keeps_pending_home() -> TestResult {
        let mut app = test_app();
        let camera = spawn_free_cam(&mut app, None);
        app.update();
        let settled = settled_pose();
        {
            let mut free_cam = app
                .world_mut()
                .get_mut::<FreeCam>(camera)
                .ok_or("camera missing FreeCam")?;
            free_cam.translate.set_target(settled.position);
            free_cam.look.set_target(settled.look);
            free_cam.roll.set_target(settled.roll);
        }

        app.update();

        assert_eq!(home_for(&app, camera)?, start_pose());
        assert!(app.world().get::<CameraHomePending>(camera).is_some());
        Ok(())
    }

    #[test]
    fn reset_free_cam_to_home_event_targets_home_pose() -> TestResult {
        let mut app = test_app();
        let camera = spawn_free_cam(&mut app, Some(app_home_pose()));
        app.update();
        set_current_pose(&mut app, camera, settled_pose())?;

        trigger_free_reset(&mut app, camera);

        let free_cam = app
            .world()
            .get::<FreeCam>(camera)
            .ok_or("camera missing FreeCam")?;
        assert_eq!(free_cam.translate.target(), app_home_pose().position);
        assert_eq!(free_cam.look.target(), app_home_pose().look);
        assert_eq!(free_cam.roll.target(), app_home_pose().roll);
        Ok(())
    }

    #[test]
    fn reset_orbit_cam_to_home_event_targets_home_pose() -> TestResult {
        let mut app = test_app();
        let home = orbit_home_pose();
        let camera = spawn_orbit_cam(&mut app, home);
        set_orbit_targets_away(&mut app, camera)?;

        trigger_orbit_reset(&mut app, camera);

        let orbit_cam = app
            .world()
            .get::<OrbitCam>(camera)
            .ok_or("camera missing OrbitCam")?;
        assert_eq!(orbit_cam.orbit.target(), home.orbit);
        assert_eq!(orbit_cam.pan.target(), home.pan);
        assert_eq!(orbit_cam.zoom.target(), home.zoom);
        Ok(())
    }

    #[test]
    fn external_reset_reports_no_physical_sources() {
        #[derive(Resource, Default)]
        struct HomedSources(Vec<InteractionSources>);

        let mut app = test_app();
        let camera = spawn_free_cam(&mut app, Some(app_home_pose()));
        app.init_resource::<HomedSources>();
        app.add_observer(
            |event: On<CameraHomed>, mut sources: ResMut<HomedSources>| {
                sources.0.push(event.sources);
            },
        );

        trigger_free_reset(&mut app, camera);

        assert_eq!(
            app.world().resource::<HomedSources>().0.as_slice(),
            &[InteractionSources::NONE]
        );
    }

    #[test]
    fn input_active_at_spawn_locks_home_immediately() -> TestResult {
        let mut app = test_app();
        app.add_systems(
            PreUpdate,
            mark_translate_active_for_free_cams.in_set(CameraInputPhase::WriteManual),
        );
        let camera = spawn_free_cam(&mut app, None);

        app.update();

        assert_eq!(home_for(&app, camera)?, start_pose());
        assert!(app.world().get::<CameraHomePending>(camera).is_none());
        Ok(())
    }

    fn mark_translate_active_for_free_cams(
        mut writer: FreeCamManualInputWriter,
        cameras: Query<Entity, With<FreeCam>>,
    ) {
        for camera in &cameras {
            if let Ok(mut input) = writer.get_mut(camera, ManualInputSource::observed_keyboard()) {
                input.mark_translate_active();
            }
        }
    }
}
