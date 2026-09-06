use bevy::input::InputSystems;
use bevy::prelude::App;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PreUpdate;
use bevy::prelude::SystemSet;
use bevy_enhanced_input::prelude::EnhancedInputSystems;

/// Public schedule phases for camera input processing.
///
/// App-authored manual camera input writers should run in
/// [`CameraInputPhase::WriteManual`].
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub enum CameraInputPhase {
    /// Library-owned preparation before enhanced-input context evaluation.
    PreInput,
    /// App-authored manual camera intent writers.
    WriteManual,
    /// Library-owned finalization before the camera controller reads input.
    Finalize,
}

/// Public `PostUpdate` set for lagrange camera controller systems.
///
/// Use this to run systems before camera controllers read input and operation
/// state, or after they write camera `Transform`s. Kind-specific public labels
/// such as [`crate::OrbitCamSystemSet`] remain available for targeting one
/// controller.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub struct CameraControllerSystemSet;

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) enum CameraInputInternalSet {
    InputModes,
    Routing,
    Installation,
    AdapterInjection,
    ActionResolution,
}

pub(crate) struct LagrangeSystemSetsPlugin;

impl Plugin for LagrangeSystemSetsPlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            PreUpdate,
            (
                CameraInputPhase::PreInput
                    .after(InputSystems)
                    .before(EnhancedInputSystems::Update),
                CameraInputPhase::WriteManual
                    .after(CameraInputPhase::PreInput)
                    .after(EnhancedInputSystems::Apply)
                    .after(CameraInputInternalSet::ActionResolution),
                CameraInputPhase::Finalize.after(CameraInputPhase::WriteManual),
            ),
        );
        app.configure_sets(
            PreUpdate,
            (
                CameraInputInternalSet::InputModes.in_set(CameraInputPhase::PreInput),
                CameraInputInternalSet::Routing
                    .in_set(CameraInputPhase::PreInput)
                    .after(CameraInputInternalSet::InputModes),
                CameraInputInternalSet::Installation
                    .in_set(CameraInputPhase::PreInput)
                    .after(CameraInputInternalSet::Routing),
                CameraInputInternalSet::AdapterInjection
                    .in_set(CameraInputPhase::PreInput)
                    .after(CameraInputInternalSet::Installation),
                CameraInputInternalSet::ActionResolution
                    .after(EnhancedInputSystems::Apply)
                    .before(CameraInputPhase::WriteManual),
            ),
        );
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
    use bevy::prelude::Component;
    use bevy::prelude::Entity;
    use bevy::prelude::In;
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
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::prelude::With;
    use bevy::prelude::World;
    use bevy::window::WindowRef;

    use super::CameraInputPhase;
    use crate::AnimationEnd;
    use crate::AnimationReason;
    use crate::CameraCommands;
    use crate::CameraInputInterruptBehavior;
    use crate::CameraInputRoutingConfig;
    use crate::CameraInputSurfaceMetrics;
    use crate::CameraMove;
    use crate::CameraPlaybackObservation;
    use crate::Focus;
    use crate::FreeCamRollTarget;
    use crate::LagrangePlugin;
    use crate::ManualInputSource;
    use crate::OrbitAngles;
    use crate::OrbitCam;
    use crate::OrbitCamInputMode;
    use crate::OrbitCamManualInputWriter;
    use crate::PlayAnimation;
    use crate::Radius;
    use crate::SequenceOwner;
    use crate::SequenceOwnership;

    const ANIMATION_RADIUS: f32 = 2.0;
    const ANIMATION_YAW: f32 = 1.0;
    const INPUT_SURFACE_SIZE: Vec2 = Vec2::new(100.0, 100.0);
    const MANUAL_ORBIT_DELTA: Vec2 = Vec2::new(25.0, 0.0);
    const MOVE_DURATION_MILLIS: u64 = 1_000;

    type TestResult = Result<(), &'static str>;

    #[derive(Component)]
    struct ScheduleInvariantCamera;

    #[derive(Resource, Default)]
    struct AnimationEventCounts {
        cancelled: usize,
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, LagrangePlugin))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>()
            .init_resource::<AnimationEventCounts>()
            .add_systems(
                PreUpdate,
                write_manual_orbit_input.in_set(CameraInputPhase::WriteManual),
            );
        app.finish();
        app
    }

    fn write_manual_orbit_input(
        mut writer: OrbitCamManualInputWriter,
        cameras: Query<Entity, With<ScheduleInvariantCamera>>,
    ) {
        let Ok(camera) = cameras.single() else {
            return;
        };
        let Ok(mut input) = writer.get_mut(camera, ManualInputSource::observed_keyboard()) else {
            return;
        };
        input.orbit(MANUAL_ORBIT_DELTA);
    }

    fn observe_animation_cancelled(world: &mut World, camera: Entity) {
        world.entity_mut(camera).observe(
            |event: On<AnimationEnd>, mut counts: ResMut<AnimationEventCounts>| {
                if matches!(event.reason, AnimationReason::Cancelled { .. }) {
                    counts.cancelled += 1;
                }
            },
        );
    }

    fn animation_move() -> CameraMove {
        CameraMove::try_to_orbital_look_at(
            Focus(Vec3::ZERO),
            OrbitAngles {
                yaw:   ANIMATION_YAW,
                pitch: 0.0,
            },
            Radius(ANIMATION_RADIUS),
            FreeCamRollTarget::InheritPrevious,
            Duration::from_millis(MOVE_DURATION_MILLIS),
            EaseFunction::Linear,
        )
        .expect("animation move is finite")
    }

    fn observe_camera_playback(
        input: In<Entity>,
        commands: CameraCommands,
    ) -> (SequenceOwnership, CameraPlaybackObservation) {
        (commands.owner(input.0), commands.observe(input.0))
    }

    fn spawn_manual_camera(app: &mut App) -> Entity {
        let mut orbit_cam = OrbitCam::default();
        orbit_cam.orbit.set_damping(0.0);
        let camera = app
            .world_mut()
            .spawn((
                ScheduleInvariantCamera,
                orbit_cam,
                OrbitCamInputMode::Manual,
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                Transform::from_xyz(0.0, 0.0, 10.0),
                CameraInputSurfaceMetrics::camera_view_and_input_surface(
                    INPUT_SURFACE_SIZE,
                    INPUT_SURFACE_SIZE,
                ),
                CameraInputInterruptBehavior::Cancel,
            ))
            .id();
        observe_animation_cancelled(app.world_mut(), camera);
        app.world_mut()
            .trigger(PlayAnimation::new(camera, [animation_move()]));
        camera
    }

    #[test]
    fn enhanced_input_scheduling_invariant() -> TestResult {
        let mut app = test_app();
        let camera = spawn_manual_camera(&mut app);
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));

        app.update();

        // The facade request becomes retained native playback before physical
        // input is evaluated, so Cancel closes it exactly once.
        assert_eq!(app.world().resource::<AnimationEventCounts>().cancelled, 1);
        let (ownership, observation) = app
            .world_mut()
            .run_system_cached_with(observe_camera_playback, camera)
            .map_err(|_| "camera playback observation system did not run")?;
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
        let orbit_cam = app
            .world()
            .get::<OrbitCam>(camera)
            .ok_or("camera missing OrbitCam")?;
        assert!(orbit_cam.orbit.target().yaw.abs() <= f32::EPSILON);
        Ok(())
    }
}
