//! Enhanced-input adapter: installs per-camera action entities and resolves them into
//! [`OrbitCamInput`] each frame.
//!
//! The work is split into three pipeline stages, each in its own submodule, scheduled in
//! distinct sets of [`CameraInputInternalSet`]:
//!
//! - [`install`] (`Installation` set) — clears stale installations, builds the action and binding
//!   entities for each camera, and gates the input context per camera.
//! - [`inject`] (`AdapterInjection` set) — each frame translates raw `bevy::input` resources (wheel
//!   / pinch / touch / button-drag) into custom inputs consumed by adapter actions.
//! - [`resolve`] (`ActionResolution` set) — reads the resulting action state and writes the
//!   per-camera [`OrbitCamInput`].
//!
//! Only [`OrbitCamInputAdapterPlugin`] is exposed outside this module.

mod inject;
mod install;
mod resolve;

use bevy::ecs::system::SystemParam;
use bevy::prelude::App;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PreUpdate;
use bevy::prelude::Vec2;
use bevy_enhanced_input::prelude::InputConditionAppExt;
use install::OrbitCamAdapterDiagnostics;
use resolve::BoolActionQueries;
use resolve::F32ActionQueries;
use resolve::Vec2ActionQueries;

use super::OrbitCamBindings;
use crate::OrbitCamKind;
use crate::input;
use crate::input::CameraBindingGateCondition;
use crate::input::CameraInputScalePolicy;
use crate::input::InteractionSources;
use crate::system_sets::CameraInputInternalSet;

pub struct OrbitCamInputAdapterPlugin;

pub(super) fn apply_scale(
    value: f32,
    scale_policy: CameraInputScalePolicy,
    slow_active: bool,
) -> f32 {
    value
        * if slow_active {
            scale_policy.slow
        } else {
            scale_policy.normal
        }
}

#[derive(Clone, Copy)]
pub(super) struct AdapterScale {
    policy:      Option<CameraInputScalePolicy>,
    slow_active: bool,
}

impl AdapterScale {
    pub(super) fn from_bindings(bindings: &OrbitCamBindings, slow_active: bool) -> Self {
        Self {
            policy: bindings.slow_mode().map(|slow_mode| slow_mode.scale),
            slow_active,
        }
    }

    pub(super) fn f32(self, value: f32) -> f32 {
        self.policy
            .map_or(value, |policy| apply_scale(value, policy, self.slow_active))
    }

    pub(super) fn vec2(self, value: Vec2) -> Vec2 {
        Vec2::new(self.f32(value.x), self.f32(value.y))
    }
}

/// Whether the home action was active last frame, so `apply_orbit_cam_home`
/// emits [`input::CameraHomed`] only when home becomes active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HomeActionState {
    Active,
    Inactive,
}

impl From<bool> for HomeActionState {
    fn from(active: bool) -> Self { if active { Self::Active } else { Self::Inactive } }
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OrbitCamAdapterFrameSources {
    orbit:       InteractionSources,
    pan:         InteractionSources,
    zoom_coarse: InteractionSources,
    zoom_smooth: InteractionSources,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OrbitCamInputActionEntities {
    orbit:               Entity,
    orbit_slow:          Entity,
    orbit_engaged:       Entity,
    pan:                 Entity,
    pan_slow:            Entity,
    pan_engaged:         Entity,
    zoom_coarse:         Entity,
    zoom_smooth:         Entity,
    zoom_smooth_slow:    Entity,
    zoom_engaged:        Entity,
    slow_mode_toggle:    Entity,
    adapter_orbit:       Entity,
    adapter_pan:         Entity,
    adapter_zoom_coarse: Entity,
    adapter_zoom_smooth: Entity,
    home:                Entity,
    orbit_sources:       InteractionSources,
    pan_sources:         InteractionSources,
    zoom_smooth_sources: InteractionSources,
    zoom_coarse_sources: InteractionSources,
    home_sources:        InteractionSources,
    home_state:          HomeActionState,
}

#[derive(SystemParam)]
pub(crate) struct OrbitCamActionQueries<'w, 's> {
    vec2:  Vec2ActionQueries<'w, 's>,
    f32:   F32ActionQueries<'w, 's>,
    bools: BoolActionQueries<'w, 's>,
}

impl Plugin for OrbitCamInputAdapterPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OrbitCamAdapterDiagnostics>()
            .add_input_condition::<install::TrackpadBindingCondition>()
            .add_input_condition::<CameraBindingGateCondition>()
            .add_systems(
                PreUpdate,
                (
                    install::clear_replaced_or_manual_installations,
                    install::install_enhanced_input_entities,
                    install::apply_context_gating,
                )
                    .chain()
                    .in_set(CameraInputInternalSet::Installation),
            )
            .add_systems(
                PreUpdate,
                inject::inject_adapter_actions.in_set(CameraInputInternalSet::AdapterInjection),
            )
            .add_systems(
                PreUpdate,
                (
                    input::resolve_actions_into_camera_input::<OrbitCamKind>,
                    resolve::apply_orbit_cam_home,
                )
                    .chain()
                    .in_set(CameraInputInternalSet::ActionResolution),
            );
    }
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use bevy::camera::RenderTarget;
    use bevy::input::gamepad::Gamepad;
    use bevy::input::gestures::PinchGesture;
    use bevy::input::mouse::AccumulatedMouseMotion;
    use bevy::input::mouse::AccumulatedMouseScroll;
    use bevy::input::mouse::MouseScrollUnit;
    use bevy::prelude::App;
    use bevy::prelude::Bundle;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::Camera;
    use bevy::prelude::Entity;
    use bevy::prelude::GamepadAxis;
    use bevy::prelude::GamepadButton;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::MouseButton;
    use bevy::prelude::On;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Time;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::prelude::World;
    use bevy::window::WindowRef;
    use bevy_enhanced_input::prelude::ActionMock;
    use bevy_enhanced_input::prelude::Binding;
    use bevy_enhanced_input::prelude::BindingOf;
    use bevy_enhanced_input::prelude::DeadZone;
    use bevy_enhanced_input::prelude::DeltaScale;
    use bevy_enhanced_input::prelude::EnhancedInputPlugin;
    use bevy_enhanced_input::prelude::InputContextAppExt;
    use bevy_enhanced_input::prelude::ModKeys;
    use bevy_enhanced_input::prelude::Scale;
    use bevy_enhanced_input::prelude::TriggerState;

    use super::OrbitCamBindings;
    use super::OrbitCamInputActionEntities;
    use super::OrbitCamInputAdapterPlugin;
    use super::inject::OrbitCamTouchAdapterOverride;
    use super::inject::TrackpadScrollTarget;
    use super::install::TrackpadBindingCondition;
    use crate::Focus;
    use crate::OrbitAngles;
    use crate::OrbitCamHomePose;
    use crate::OrbitCamKind;
    use crate::Radius;
    use crate::input;
    use crate::input::BUTTON_ZOOM_SCALE;
    use crate::input::CameraBindingGateCondition;
    use crate::input::CameraHomed;
    use crate::input::CameraInputDisabled;
    use crate::input::CameraInputGamepadSelectionPolicy;
    use crate::input::CameraInputModeReplaced;
    use crate::input::CameraInputModesPlugin;
    use crate::input::CameraInputRoutingConfig;
    use crate::input::CameraInputRoutingPlugin;
    use crate::input::ControlSpeed;
    use crate::input::DISABLED_INPUT_GAIN;
    use crate::input::HeldBinding;
    use crate::input::INVALID_SOURCE_INPUT_GAIN;
    use crate::input::InputBinding;
    use crate::input::InteractionSources;
    use crate::input::OneFingerGestures;
    use crate::input::OrbitCamBlenderLikePreset;
    use crate::input::OrbitCamButtonDragZoom;
    use crate::input::OrbitCamButtonDragZoomAxis;
    use crate::input::OrbitCamGamepadPreset;
    use crate::input::OrbitCamInputContext;
    use crate::input::OrbitCamInputGain;
    use crate::input::OrbitCamInputMode;
    use crate::input::OrbitCamLineScroll;
    use crate::input::OrbitCamMouseDrag;
    use crate::input::OrbitCamMouseWheelZoom;
    use crate::input::OrbitCamPinchZoom;
    use crate::input::OrbitCamPreset;
    use crate::input::OrbitCamTouchBinding;
    use crate::input::OrbitCamTrackpadScroll;
    use crate::input::PINCH_GESTURE_AMPLIFICATION;
    use crate::input::PINCH_INPUT_GAIN;
    use crate::input::PIXEL_SCROLL_SCALE;
    use crate::input::TOUCH_PINCH_SCALE;
    use crate::input::TouchGestures;
    use crate::input::TouchTracker;
    use crate::input::TwoFingerGestures;
    use crate::input::WHEEL_INPUT_GAIN;
    use crate::input::ZoomInversion;
    use crate::orbit_cam::OrbitCam;
    use crate::orbit_cam::OrbitCamInput;
    use crate::orbit_cam::OrbitDelta;
    use crate::system_sets::LagrangeSystemSetsPlugin;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            EnhancedInputPlugin,
            LagrangeSystemSetsPlugin,
            CameraInputModesPlugin,
            CameraInputRoutingPlugin,
            OrbitCamInputAdapterPlugin,
        ))
        .add_input_context::<OrbitCamInputContext>();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>()
            .init_resource::<TouchTracker>()
            .add_message::<PinchGesture>();
        crate::camera_home::add_orbit_cam_home_reset_systems(&mut app);
        app.finish();
        app
    }

    fn spawn_camera(world: &mut World, components: impl Bundle) -> Entity {
        world
            .spawn((
                OrbitCam::default(),
                OrbitCamInput::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                components,
            ))
            .id()
    }

    fn route_to(app: &mut App, camera: Entity) {
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
    }

    type TestResult = Result<(), &'static str>;

    #[derive(Resource, Default)]
    struct ModeReplacementEvents(usize);

    const BUTTON_DRAG_MOTION_Y: f32 = -8.0;
    const BUTTON_DRAG_INPUT_GAIN: f32 = 0.25;
    const HOME_FOCUS: Vec3 = Vec3::new(1.0, 2.0, 3.0);
    const HOME_ORBIT_PITCH: f32 = 0.25;
    const HOME_ORBIT_YAW: f32 = 0.5;
    const HOME_ZOOM_RADIUS: f32 = 4.0;
    const NON_HOME_FOCUS: Vec3 = Vec3::new(9.0, 9.0, 9.0);
    const NON_HOME_ORBIT_PITCH: f32 = 1.0;
    const NON_HOME_ORBIT_YAW: f32 = 2.0;
    const NON_HOME_ZOOM_RADIUS: f32 = 8.0;
    const PINCH_DELTA: f32 = 2.0;
    const PIXEL_SCROLL_DELTA_Y: f32 = 20.0;
    const SLOW_SCALE: f32 = 0.5;
    const TOUCH_MOTION_X: f32 = 6.0;
    const TOUCH_MOTION_Y: f32 = 8.0;
    const TOUCH_ORBIT_INPUT_GAIN: f32 = 0.25;
    const TOUCH_PAN_INPUT_GAIN: f32 = 0.5;
    const TOUCH_PINCH_DELTA: f32 = 4.0;
    const TOUCH_ZOOM_INPUT_GAIN: f32 = 0.75;
    const TRACKPAD_DUPLICATE_INPUT_GAIN: f32 = 0.5;
    const TRACKPAD_ORBIT_PRIORITY_INPUT_GAIN: f32 = 3.0;
    const TRACKPAD_PAN_PRIORITY_INPUT_GAIN: f32 = 2.0;
    const TRACKPAD_ZOOM_INPUT_GAIN: f32 = 0.25;
    const WHEEL_SCROLL_DELTA: f32 = 6.0;

    fn camera_input(app: &App, camera: Entity) -> Result<&OrbitCamInput, &'static str> {
        app.world()
            .get::<OrbitCamInput>(camera)
            .ok_or("camera should have OrbitCamInput")
    }

    fn assert_f32_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() <= f32::EPSILON);
    }

    fn assert_no_camera_input(app: &App, camera: Entity) -> TestResult {
        let input = camera_input(app, camera)?;
        assert!(!input.has_input());
        assert!(input.sources().is_empty());
        Ok(())
    }

    fn installed_scale_for(
        app: &App,
        camera: Entity,
        matches_binding: impl Fn(&Binding) -> bool,
    ) -> Option<Vec3> {
        input::installed_input_entities_for::<OrbitCamKind>(app.world(), camera)
            .into_iter()
            .find_map(|entity| {
                let binding = app.world().get::<Binding>(entity)?;
                let scale = app.world().get::<Scale>(entity)?;
                matches_binding(binding).then_some(scale.factor)
            })
    }

    fn trackpad_scale_for_index(app: &App, camera: Entity, index: usize) -> Option<Vec3> {
        input::installed_input_entities_for::<OrbitCamKind>(app.world(), camera)
            .into_iter()
            .find_map(|entity| {
                let condition = app.world().get::<TrackpadBindingCondition>(entity)?;
                let scale = app.world().get::<Scale>(entity)?;
                (condition.index == index).then_some(scale.factor)
            })
    }

    fn spawn_slow_blender_like_camera(app: &mut App) -> Result<Entity, &'static str> {
        let bindings = OrbitCamBlenderLikePreset::default()
            .slow_scale(SLOW_SCALE)
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(app, camera);
        Ok(camera)
    }

    /// Fires the Alt+S slow-mode toggle edge and releases the keys, leaving the
    /// camera's slow latch active for the caller's following update. Releasing
    /// the modifier keeps a later mouse drag from being read as a gated gesture.
    fn press_slow_toggle(app: &mut App) {
        {
            let mut keyboard = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keyboard.press(KeyCode::AltLeft);
            keyboard.press(KeyCode::KeyS);
        }
        app.update();
        let mut keyboard = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keyboard.release(KeyCode::AltLeft);
        keyboard.release(KeyCode::KeyS);
    }

    #[test]
    fn installer_replaces_placeholder_with_action_entities() {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        route_to(&mut app, camera);

        app.update();

        assert!(app.world().get::<OrbitCamInputContext>(camera).is_some());
        assert!(
            app.world()
                .get::<OrbitCamInputActionEntities>(camera)
                .is_some()
        );
        assert!(input::installed_input_entities_for::<OrbitCamKind>(app.world(), camera).len() > 1);
        assert!(
            !input::input_installation_has_placeholder_for::<OrbitCamKind>(app.world(), camera)
        );
    }

    #[test]
    fn installer_binds_adapter_actions_to_custom_inputs() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        route_to(&mut app, camera);

        app.update();

        let actions = *app
            .world()
            .get::<OrbitCamInputActionEntities>(camera)
            .ok_or("adapter actions should be installed")?;
        let custom_bound_actions =
            input::installed_input_entities_for::<OrbitCamKind>(app.world(), camera)
                .into_iter()
                .filter_map(|entity| {
                    let binding = app.world().get::<Binding>(entity)?;
                    let binding_of = app.world().get::<BindingOf>(entity)?;
                    matches!(binding, Binding::Custom(_)).then_some(**binding_of)
                })
                .collect::<Vec<_>>();
        let trackpad_custom_bindings =
            input::installed_input_entities_for::<OrbitCamKind>(app.world(), camera)
                .into_iter()
                .filter(|entity| {
                    app.world()
                        .get::<Binding>(*entity)
                        .is_some_and(|binding| matches!(binding, Binding::Custom(_)))
                })
                .filter(|entity| {
                    app.world()
                        .get::<TrackpadBindingCondition>(*entity)
                        .is_some()
                })
                .count();

        assert_eq!(custom_bound_actions.len(), 5);
        assert!(custom_bound_actions.contains(&actions.adapter_orbit));
        assert!(custom_bound_actions.contains(&actions.adapter_pan));
        assert!(custom_bound_actions.contains(&actions.adapter_zoom_coarse));
        assert!(custom_bound_actions.contains(&actions.adapter_zoom_smooth));
        assert_eq!(trackpad_custom_bindings, 1);
        Ok(())
    }

    #[test]
    fn duplicate_smooth_scroll_bindings_install_distinct_conditions() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(OrbitCamTrackpadScroll::default())
            .zoom(OrbitCamTrackpadScroll::default().with_input_gain(TRACKPAD_DUPLICATE_INPUT_GAIN))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);

        app.update();

        let mut condition_indexes =
            input::installed_input_entities_for::<OrbitCamKind>(app.world(), camera)
                .into_iter()
                .filter_map(|entity| {
                    app.world()
                        .get::<TrackpadBindingCondition>(entity)
                        .map(|condition| condition.index)
                })
                .collect::<Vec<_>>();
        condition_indexes.sort_unstable();

        assert_eq!(condition_indexes, vec![0, 1]);
        Ok(())
    }

    #[test]
    fn native_install_uses_composed_scale_and_input_gain() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .orbit(HeldBinding::new(
                InputBinding::gamepad_axes_2d(GamepadAxis::RightStickX, GamepadAxis::RightStickY)
                    .with_scale(Vec2::new(-2.0, 4.0))
                    .with_input_gain(0.25),
                GamepadButton::RightTrigger2,
            ))
            .gamepad(CameraInputGamepadSelectionPolicy::Active)
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);

        app.update();

        let x_scale = installed_scale_for(&app, camera, |binding| {
            matches!(binding, Binding::GamepadAxis(GamepadAxis::RightStickX))
        })
        .ok_or("right stick x binding should have a scale")?;
        let y_scale = installed_scale_for(&app, camera, |binding| {
            matches!(binding, Binding::GamepadAxis(GamepadAxis::RightStickY))
        })
        .ok_or("right stick y binding should have a scale")?;

        assert_eq!(x_scale, Vec3::splat(-0.5));
        assert_eq!(y_scale, Vec3::splat(1.0));
        Ok(())
    }

    #[test]
    fn zero_sensitive_native_held_binding_is_not_installed_or_attributed() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .orbit(OrbitCamMouseDrag::new(MouseButton::Left).with_input_gain(DISABLED_INPUT_GAIN))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);

        app.update();

        let installed = input::installed_input_entities_for::<OrbitCamKind>(app.world(), camera);
        assert!(installed.iter().all(|entity| {
            !matches!(
                app.world().get::<Binding>(*entity),
                Some(Binding::MouseMotion { .. } | Binding::MouseButton { .. })
            )
        }));
        let actions = app
            .world()
            .get::<OrbitCamInputActionEntities>(camera)
            .ok_or("input actions should be installed")?;
        assert!(actions.orbit_sources.is_empty());
        Ok(())
    }

    #[test]
    fn mouse_drag_action_resolves_to_orbit_input() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        route_to(&mut app, camera);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(5.0, -2.0);

        app.update();

        let input = camera_input(&app, camera)?;
        assert_eq!(input.orbit(), OrbitDelta::from(Vec2::new(5.0, -2.0)));
        assert!(input.has_orbit());
        assert!(input.sources().contains(InteractionSources::MOUSE));
        Ok(())
    }

    #[test]
    fn home_action_resets_targets_to_home_pose() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .home(KeyCode::KeyH)
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        let home_pose = OrbitCamHomePose {
            orbit: OrbitAngles {
                yaw:   HOME_ORBIT_YAW,
                pitch: HOME_ORBIT_PITCH,
            },
            pan:   Focus(HOME_FOCUS),
            zoom:  Radius(HOME_ZOOM_RADIUS),
        };
        app.world_mut().entity_mut(camera).insert(home_pose);
        route_to(&mut app, camera);
        app.update();

        {
            let mut orbit_cam = app
                .world_mut()
                .get_mut::<OrbitCam>(camera)
                .ok_or("camera should have OrbitCam")?;
            orbit_cam.orbit.set_target(OrbitAngles {
                yaw:   NON_HOME_ORBIT_YAW,
                pitch: NON_HOME_ORBIT_PITCH,
            });
            orbit_cam.pan.set_target(Focus(NON_HOME_FOCUS));
            orbit_cam.zoom.set_target(Radius(NON_HOME_ZOOM_RADIUS));
        }

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyH);
        app.update();

        let orbit_cam = app
            .world()
            .get::<OrbitCam>(camera)
            .ok_or("camera should have OrbitCam")?;
        assert_eq!(orbit_cam.orbit.target(), home_pose.orbit);
        assert_eq!(orbit_cam.pan.target(), home_pose.pan);
        assert_eq!(orbit_cam.zoom.target(), home_pose.zoom);
        Ok(())
    }

    #[test]
    fn home_action_emits_camera_homed_on_rising_edge() -> TestResult {
        #[derive(Resource, Default)]
        struct CameraHomedEvents(Vec<InteractionSources>);

        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .home(KeyCode::KeyH)
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        app.world_mut().entity_mut(camera).insert(OrbitCamHomePose {
            orbit: OrbitAngles {
                yaw:   0.0,
                pitch: 0.0,
            },
            pan:   Focus(Vec3::ZERO),
            zoom:  Radius(1.0),
        });
        route_to(&mut app, camera);
        app.init_resource::<CameraHomedEvents>();
        app.add_observer(
            |event: On<CameraHomed>, mut events: ResMut<CameraHomedEvents>| {
                events.0.push(event.sources);
            },
        );
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyH);
        app.update();
        app.update();

        assert_eq!(
            app.world().resource::<CameraHomedEvents>().0.as_slice(),
            &[InteractionSources::KEYBOARD]
        );
        Ok(())
    }

    #[test]
    fn home_action_reports_only_keyboard_when_keyboard_binding_fires() -> TestResult {
        #[derive(Resource, Default)]
        struct CameraHomedEvents(Vec<InteractionSources>);

        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .home(KeyCode::KeyH)
            .home(GamepadButton::Select)
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        app.world_mut().entity_mut(camera).insert(OrbitCamHomePose {
            orbit: OrbitAngles {
                yaw:   0.0,
                pitch: 0.0,
            },
            pan:   Focus(Vec3::ZERO),
            zoom:  Radius(1.0),
        });
        route_to(&mut app, camera);
        app.init_resource::<CameraHomedEvents>();
        app.add_observer(
            |event: On<CameraHomed>, mut events: ResMut<CameraHomedEvents>| {
                events.0.push(event.sources);
            },
        );
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyH);
        app.update();

        assert_eq!(
            app.world().resource::<CameraHomedEvents>().0.as_slice(),
            &[InteractionSources::KEYBOARD]
        );
        Ok(())
    }

    #[test]
    fn home_action_reports_static_union_when_no_binding_attributes() -> TestResult {
        #[derive(Resource, Default)]
        struct CameraHomedEvents(Vec<InteractionSources>);

        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .home(KeyCode::KeyH)
            .home(GamepadButton::Select)
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        app.world_mut().entity_mut(camera).insert(OrbitCamHomePose {
            orbit: OrbitAngles {
                yaw:   0.0,
                pitch: 0.0,
            },
            pan:   Focus(Vec3::ZERO),
            zoom:  Radius(1.0),
        });
        route_to(&mut app, camera);
        app.init_resource::<CameraHomedEvents>();
        app.add_observer(
            |event: On<CameraHomed>, mut events: ResMut<CameraHomedEvents>| {
                events.0.push(event.sources);
            },
        );
        app.update();

        let actions = *app
            .world()
            .get::<OrbitCamInputActionEntities>(camera)
            .ok_or("adapter actions should be installed")?;
        app.world_mut()
            .entity_mut(actions.home)
            .insert(ActionMock::once(TriggerState::Fired, true));
        app.update();

        assert_eq!(
            app.world().resource::<CameraHomedEvents>().0.as_slice(),
            &[InteractionSources::KEYBOARD.union(InteractionSources::GAMEPAD)]
        );
        Ok(())
    }

    #[test]
    fn blender_like_shift_middle_mouse_resolves_to_pan_only() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        );
        route_to(&mut app, camera);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(5.0, -2.0);

        app.update();

        let input = camera_input(&app, camera)?;
        assert!(!input.has_orbit());
        assert_eq!(input.pan().pixels(), Vec2::new(5.0, -2.0));
        assert!(input.sources().contains(InteractionSources::MOUSE));
        Ok(())
    }

    #[test]
    fn forwarded_line_scroll_routes_modifiers_without_wheel_zoom() -> TestResult {
        let gain = OrbitCamInputGain::new().orbit(12.0).pan(8.0).zoom(20.0);
        for (keys, target) in [
            (vec![], TrackpadScrollTarget::Orbit),
            (vec![KeyCode::ShiftLeft], TrackpadScrollTarget::Pan),
            (vec![KeyCode::ControlLeft], TrackpadScrollTarget::Zoom),
            (
                vec![KeyCode::ControlLeft, KeyCode::ShiftLeft],
                TrackpadScrollTarget::Zoom,
            ),
        ] {
            let mut app = test_app();
            let bindings = OrbitCamBlenderLikePreset::default()
                .line_scroll_input_gain(Some(gain))
                .build()
                .map_err(|_| "bindings should validate")?;
            let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
            route_to(&mut app, camera);
            for key in keys {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(key);
            }
            let delta = Vec2::new(2.0, 3.0);
            *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
                unit: MouseScrollUnit::Line,
                delta,
            };
            app.update();
            let input = camera_input(&app, camera)?;
            assert_eq!(input.has_orbit(), target == TrackpadScrollTarget::Orbit);
            assert_eq!(input.has_pan(), target == TrackpadScrollTarget::Pan);
            assert_eq!(input.has_zoom(), target == TrackpadScrollTarget::Zoom);
            assert_f32_close(input.zoom_coarse().amount(), 0.0);
            match target {
                TrackpadScrollTarget::Orbit => assert_eq!(
                    input.orbit(),
                    OrbitDelta::from(delta * gain.orbit_input_gain().value())
                ),
                TrackpadScrollTarget::Pan => {
                    assert_eq!(input.pan().pixels(), delta * gain.pan_input_gain().value());
                },
                TrackpadScrollTarget::Zoom => assert_f32_close(
                    input.zoom_smooth().amount(),
                    delta.y * gain.zoom_input_gain().value() * PIXEL_SCROLL_SCALE,
                ),
            }
            assert!(input.sources().contains(InteractionSources::WHEEL));
            assert!(!input.sources().contains(InteractionSources::SMOOTH_SCROLL));
        }
        Ok(())
    }

    #[test]
    fn forwarded_mode_preserves_pixel_scroll_and_respects_camera_gating() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBlenderLikePreset::default()
            .line_scroll_input_gain(Some(OrbitCamInputGain::uniform(20.0)))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        let delta = Vec2::new(4.0, 6.0);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit: MouseScrollUnit::Pixel,
            delta,
        };
        app.update();
        assert_eq!(camera_input(&app, camera)?.orbit(), OrbitDelta::from(delta));
        assert!(
            camera_input(&app, camera)?
                .sources()
                .contains(InteractionSources::SMOOTH_SCROLL)
        );
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit: MouseScrollUnit::Line,
            delta,
        };
        app.update();
        assert!(camera_input(&app, camera)?.has_orbit());
        app.world_mut()
            .entity_mut(camera)
            .insert(CameraInputDisabled);
        app.update();
        assert_no_camera_input(&app, camera)
    }

    #[test]
    fn line_scroll_zoom_inverts_and_mode_changes_remove_old_bindings() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(OrbitCamLineScroll::default().with_input_gain(WHEEL_INPUT_GAIN))
            .zoom_inversion(ZoomInversion::Inverted)
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Line,
            delta: Vec2::new(0.0, WHEEL_SCROLL_DELTA),
        };
        app.update();
        assert_f32_close(
            camera_input(&app, camera)?.zoom_smooth().amount(),
            -WHEEL_SCROLL_DELTA * WHEEL_INPUT_GAIN * PIXEL_SCROLL_SCALE,
        );
        app.world_mut()
            .entity_mut(camera)
            .insert(OrbitCamInputMode::with_preset(
                OrbitCamPreset::blender_like(),
            ));
        app.update();
        assert_f32_close(
            camera_input(&app, camera)?.zoom_coarse().amount(),
            WHEEL_SCROLL_DELTA,
        );
        assert_f32_close(camera_input(&app, camera)?.zoom_smooth().amount(), 0.0);
        Ok(())
    }

    #[test]
    fn custom_line_binding_overrides_wheel_only_when_modifiers_match() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .orbit(OrbitCamLineScroll::default().with_mod_keys(ModKeys::SHIFT))
            .zoom(OrbitCamMouseWheelZoom)
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Line,
            delta: Vec2::new(2.0, 3.0),
        };
        app.update();
        assert!(camera_input(&app, camera)?.has_zoom());
        assert!(!camera_input(&app, camera)?.has_orbit());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        app.update();
        assert!(camera_input(&app, camera)?.has_orbit());
        assert!(!camera_input(&app, camera)?.has_zoom());
        Ok(())
    }

    #[test]
    fn wheel_line_adapter_resolves_to_coarse_zoom() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        route_to(&mut app, camera);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Line,
            delta: Vec2::new(0.0, 3.0),
        };

        app.update();

        let input = camera_input(&app, camera)?;
        assert_f32_close(input.zoom_coarse().amount(), 3.0);
        assert!(input.has_zoom());
        assert!(input.sources().contains(InteractionSources::WHEEL));
        Ok(())
    }

    #[test]
    fn tuned_wheel_line_adapter_scales_and_reports_wheel() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(OrbitCamMouseWheelZoom.with_input_gain(WHEEL_INPUT_GAIN))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Line,
            delta: Vec2::new(0.0, WHEEL_SCROLL_DELTA),
        };

        app.update();

        let input = camera_input(&app, camera)?;
        assert_f32_close(
            input.zoom_coarse().amount(),
            WHEEL_SCROLL_DELTA * WHEEL_INPUT_GAIN,
        );
        assert!(input.sources().contains(InteractionSources::WHEEL));
        Ok(())
    }

    #[test]
    fn zero_sensitive_wheel_zoom_is_inactive() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(OrbitCamMouseWheelZoom.with_input_gain(DISABLED_INPUT_GAIN))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Line,
            delta: Vec2::new(0.0, WHEEL_SCROLL_DELTA),
        };

        app.update();

        assert_no_camera_input(&app, camera)
    }

    #[test]
    fn blender_like_trackpad_shift_resolves_to_pan_only() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        );
        route_to(&mut app, camera);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Pixel,
            delta: Vec2::new(4.0, 6.0),
        };

        app.update();

        let input = camera_input(&app, camera)?;
        assert!(!input.has_orbit());
        assert_eq!(input.pan().pixels(), Vec2::new(4.0, 6.0));
        assert!(input.sources().contains(InteractionSources::SMOOTH_SCROLL));
        Ok(())
    }

    #[test]
    fn routed_second_blender_like_trackpad_resolves_to_orbit_only() -> TestResult {
        let mut app = test_app();
        let primary = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        );
        let second = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        );
        route_to(&mut app, second);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Pixel,
            delta: Vec2::new(4.0, 6.0),
        };

        app.update();

        assert!(!camera_input(&app, primary)?.has_input());
        let second_input = camera_input(&app, second)?;
        assert_eq!(second_input.orbit(), OrbitDelta::from(Vec2::new(4.0, 6.0)));
        assert!(
            second_input
                .sources()
                .contains(InteractionSources::SMOOTH_SCROLL)
        );
        Ok(())
    }

    #[test]
    fn blender_like_trackpad_control_resolves_to_zoom_only() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        );
        route_to(&mut app, camera);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ControlLeft);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Pixel,
            delta: Vec2::new(4.0, 6.0),
        };

        app.update();

        let input = camera_input(&app, camera)?;
        assert!(!input.has_orbit());
        assert!(!input.has_pan());
        assert_f32_close(input.zoom_smooth().amount(), 6.0 * PIXEL_SCROLL_SCALE);
        assert!(input.sources().contains(InteractionSources::SMOOTH_SCROLL));
        Ok(())
    }

    #[test]
    fn blender_like_slow_latch_scales_mouse_drag_orbit() -> TestResult {
        let mut app = test_app();
        let camera = spawn_slow_blender_like_camera(&mut app)?;
        press_slow_toggle(&mut app);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(8.0, -4.0);

        app.update();

        assert_eq!(
            camera_input(&app, camera)?.orbit(),
            OrbitDelta::from(Vec2::new(4.0, -2.0))
        );
        Ok(())
    }

    #[test]
    fn blender_like_slow_toggle_does_not_repeat_while_held() -> TestResult {
        let mut app = test_app();
        let camera = spawn_slow_blender_like_camera(&mut app)?;
        {
            let mut keyboard = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keyboard.press(KeyCode::AltLeft);
            keyboard.press(KeyCode::KeyS);
        }

        // Hold the combo across two updates: the Press edge fires on the first,
        // and the latch must stay active rather than toggle back off on the second.
        app.update();
        app.update();

        {
            let mut keyboard = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keyboard.release(KeyCode::AltLeft);
            keyboard.release(KeyCode::KeyS);
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(8.0, -4.0);

        app.update();

        assert_eq!(
            camera_input(&app, camera)?.orbit(),
            OrbitDelta::from(Vec2::new(4.0, -2.0))
        );
        Ok(())
    }

    #[test]
    fn blender_like_slow_latch_scales_mouse_drag_pan() -> TestResult {
        let mut app = test_app();
        let camera = spawn_slow_blender_like_camera(&mut app)?;
        press_slow_toggle(&mut app);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(8.0, -4.0);

        app.update();

        assert_eq!(
            camera_input(&app, camera)?.pan().pixels(),
            Vec2::new(4.0, -2.0)
        );
        Ok(())
    }

    #[test]
    fn blender_like_slow_latch_scales_trackpad_orbit() -> TestResult {
        let mut app = test_app();
        let camera = spawn_slow_blender_like_camera(&mut app)?;
        press_slow_toggle(&mut app);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Pixel,
            delta: Vec2::new(4.0, 6.0),
        };

        app.update();

        assert_eq!(
            camera_input(&app, camera)?.orbit(),
            OrbitDelta::from(Vec2::new(2.0, 3.0))
        );
        Ok(())
    }

    #[test]
    fn blender_like_slow_latch_scales_trackpad_pan() -> TestResult {
        let mut app = test_app();
        let camera = spawn_slow_blender_like_camera(&mut app)?;
        press_slow_toggle(&mut app);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Pixel,
            delta: Vec2::new(4.0, 6.0),
        };

        app.update();

        assert_eq!(
            camera_input(&app, camera)?.pan().pixels(),
            Vec2::new(2.0, 3.0)
        );
        Ok(())
    }

    #[test]
    fn blender_like_slow_latch_scales_trackpad_zoom() -> TestResult {
        let mut app = test_app();
        let camera = spawn_slow_blender_like_camera(&mut app)?;
        press_slow_toggle(&mut app);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ControlLeft);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Pixel,
            delta: Vec2::new(4.0, 6.0),
        };

        app.update();

        assert_f32_close(
            camera_input(&app, camera)?.zoom_smooth().amount(),
            6.0 * PIXEL_SCROLL_SCALE * SLOW_SCALE,
        );
        Ok(())
    }

    #[test]
    fn pixel_scroll_adapter_resolves_to_smooth_zoom() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(OrbitCamTrackpadScroll::default())
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Pixel,
            delta: Vec2::new(0.0, 20.0),
        };

        app.update();

        let input = camera_input(&app, camera)?;
        assert_f32_close(input.zoom_smooth().amount(), 20.0 * PIXEL_SCROLL_SCALE);
        assert!(input.sources().contains(InteractionSources::SMOOTH_SCROLL));
        Ok(())
    }

    #[test]
    fn pixel_scroll_zoom_applies_selected_input_gain_once() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(OrbitCamTrackpadScroll::default().with_input_gain(TRACKPAD_ZOOM_INPUT_GAIN))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Pixel,
            delta: Vec2::new(0.0, PIXEL_SCROLL_DELTA_Y),
        };

        app.update();

        let input = camera_input(&app, camera)?;
        assert_f32_close(
            input.zoom_smooth().amount(),
            PIXEL_SCROLL_DELTA_Y * PIXEL_SCROLL_SCALE * TRACKPAD_ZOOM_INPUT_GAIN,
        );
        assert!(input.sources().contains(InteractionSources::SMOOTH_SCROLL));
        Ok(())
    }

    #[test]
    fn duplicate_smooth_scroll_zoom_selects_highest_enabled_index_scale() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(OrbitCamTrackpadScroll::default().with_input_gain(WHEEL_INPUT_GAIN))
            .zoom(OrbitCamTrackpadScroll::default().with_input_gain(TRACKPAD_DUPLICATE_INPUT_GAIN))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Pixel,
            delta: Vec2::new(0.0, PIXEL_SCROLL_DELTA_Y),
        };

        app.update();

        assert_eq!(
            trackpad_scale_for_index(&app, camera, 1),
            Some(Vec3::splat(
                PIXEL_SCROLL_SCALE * TRACKPAD_DUPLICATE_INPUT_GAIN
            ))
        );
        assert_f32_close(
            camera_input(&app, camera)?.zoom_smooth().amount(),
            PIXEL_SCROLL_DELTA_Y * PIXEL_SCROLL_SCALE * TRACKPAD_DUPLICATE_INPUT_GAIN,
        );
        Ok(())
    }

    #[test]
    fn zero_sensitive_smooth_scroll_is_not_installed_or_active() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(OrbitCamTrackpadScroll::default().with_input_gain(DISABLED_INPUT_GAIN))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Pixel,
            delta: Vec2::new(0.0, PIXEL_SCROLL_DELTA_Y),
        };

        app.update();

        let trackpad_conditions =
            input::installed_input_entities_for::<OrbitCamKind>(app.world(), camera)
                .into_iter()
                .filter(|entity| {
                    app.world()
                        .get::<TrackpadBindingCondition>(*entity)
                        .is_some()
                })
                .count();
        assert_eq!(trackpad_conditions, 0);
        assert_no_camera_input(&app, camera)
    }

    #[test]
    fn smooth_scroll_target_priority_ignores_input_gain() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .orbit(
                OrbitCamTrackpadScroll::default()
                    .with_input_gain(TRACKPAD_ORBIT_PRIORITY_INPUT_GAIN),
            )
            .pan(
                OrbitCamTrackpadScroll::default().with_input_gain(TRACKPAD_PAN_PRIORITY_INPUT_GAIN),
            )
            .zoom(OrbitCamTrackpadScroll::default().with_input_gain(TRACKPAD_ZOOM_INPUT_GAIN))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Pixel,
            delta: Vec2::new(TOUCH_MOTION_X, PIXEL_SCROLL_DELTA_Y),
        };

        app.update();

        let input = camera_input(&app, camera)?;
        assert!(!input.has_orbit());
        assert!(!input.has_pan());
        assert_f32_close(
            input.zoom_smooth().amount(),
            PIXEL_SCROLL_DELTA_Y * PIXEL_SCROLL_SCALE * TRACKPAD_ZOOM_INPUT_GAIN,
        );
        Ok(())
    }

    #[test]
    fn smooth_scroll_modifier_count_wins_before_target_priority() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .pan(
                OrbitCamTrackpadScroll::default()
                    .with_mod_keys(ModKeys::SHIFT | ModKeys::CONTROL)
                    .with_input_gain(TRACKPAD_PAN_PRIORITY_INPUT_GAIN),
            )
            .zoom(
                OrbitCamTrackpadScroll::default()
                    .with_mod_keys(ModKeys::CONTROL)
                    .with_input_gain(TRACKPAD_ZOOM_INPUT_GAIN),
            )
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        {
            let mut keyboard = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keyboard.press(KeyCode::ControlLeft);
            keyboard.press(KeyCode::ShiftLeft);
        }
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Pixel,
            delta: Vec2::new(TOUCH_MOTION_X, TOUCH_MOTION_Y),
        };

        app.update();

        let input = camera_input(&app, camera)?;
        assert_eq!(
            input.pan().pixels(),
            Vec2::new(TOUCH_MOTION_X, TOUCH_MOTION_Y) * TRACKPAD_PAN_PRIORITY_INPUT_GAIN
        );
        assert!(!input.has_zoom());
        assert!(input.sources().contains(InteractionSources::SMOOTH_SCROLL));
        Ok(())
    }

    #[test]
    fn pinch_adapter_resolves_to_smooth_zoom() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        route_to(&mut app, camera);
        app.world_mut().write_message(PinchGesture(2.0));

        app.update();

        let input = camera_input(&app, camera)?;
        assert_f32_close(
            input.zoom_smooth().amount(),
            2.0 * PINCH_GESTURE_AMPLIFICATION,
        );
        assert!(input.sources().contains(InteractionSources::PINCH));
        Ok(())
    }

    #[test]
    fn tuned_pinch_adapter_scales_and_reports_pinch() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(OrbitCamPinchZoom.with_input_gain(PINCH_INPUT_GAIN))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        app.world_mut().write_message(PinchGesture(PINCH_DELTA));

        app.update();

        let input = camera_input(&app, camera)?;
        assert_f32_close(
            input.zoom_smooth().amount(),
            PINCH_DELTA * PINCH_GESTURE_AMPLIFICATION * PINCH_INPUT_GAIN,
        );
        assert!(input.sources().contains(InteractionSources::PINCH));
        Ok(())
    }

    #[test]
    fn zero_sensitive_pinch_zoom_is_inactive() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(OrbitCamPinchZoom.with_input_gain(DISABLED_INPUT_GAIN))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        app.world_mut().write_message(PinchGesture(PINCH_DELTA));

        app.update();

        assert_no_camera_input(&app, camera)
    }

    #[test]
    fn blender_like_slow_latch_scales_wheel_zoom() -> TestResult {
        let mut app = test_app();
        let camera = spawn_slow_blender_like_camera(&mut app)?;
        press_slow_toggle(&mut app);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Line,
            delta: Vec2::new(0.0, 6.0),
        };

        app.update();

        assert_f32_close(camera_input(&app, camera)?.zoom_coarse().amount(), 3.0);
        Ok(())
    }

    #[test]
    fn blender_like_slow_latch_scales_pinch_zoom() -> TestResult {
        let mut app = test_app();
        let camera = spawn_slow_blender_like_camera(&mut app)?;
        press_slow_toggle(&mut app);
        app.world_mut().write_message(PinchGesture(2.0));

        app.update();

        assert_f32_close(
            camera_input(&app, camera)?.zoom_smooth().amount(),
            2.0 * PINCH_GESTURE_AMPLIFICATION * SLOW_SCALE,
        );
        Ok(())
    }

    #[test]
    fn pinch_adapter_is_suppressed_by_routed_held_action() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        route_to(&mut app, camera);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut().write_message(PinchGesture(2.0));

        app.update();

        let input = camera_input(&app, camera)?;
        assert_f32_close(input.zoom_smooth().amount(), 0.0);
        assert!(!input.sources().contains(InteractionSources::PINCH));
        Ok(())
    }

    #[test]
    fn zero_sensitive_pan_binding_does_not_override_orbit() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .orbit(OrbitCamMouseDrag::new(MouseButton::Left))
            .pan(
                OrbitCamMouseDrag::new(MouseButton::Left)
                    .with_mod_keys(ModKeys::SHIFT)
                    .with_input_gain(DISABLED_INPUT_GAIN),
            )
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(5.0, -2.0);

        app.update();

        let input = camera_input(&app, camera)?;
        assert_eq!(input.orbit(), OrbitDelta::from(Vec2::new(5.0, -2.0)));
        assert!(!input.has_pan());
        assert!(input.sources().contains(InteractionSources::MOUSE));
        Ok(())
    }

    #[test]
    fn zero_sensitive_held_binding_does_not_suppress_pinch() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .orbit(OrbitCamMouseDrag::new(MouseButton::Left).with_input_gain(DISABLED_INPUT_GAIN))
            .zoom(OrbitCamPinchZoom)
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut().write_message(PinchGesture(2.0));

        app.update();

        let input = camera_input(&app, camera)?;
        assert_f32_close(
            input.zoom_smooth().amount(),
            2.0 * PINCH_GESTURE_AMPLIFICATION,
        );
        assert!(input.sources().contains(InteractionSources::PINCH));
        Ok(())
    }

    #[test]
    fn blender_like_shift_modifier_suppresses_pinch() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        );
        route_to(&mut app, camera);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        app.world_mut().write_message(PinchGesture(2.0));

        app.update();

        let input = camera_input(&app, camera)?;
        assert_f32_close(input.zoom_smooth().amount(), 0.0);
        assert!(!input.sources().contains(InteractionSources::PINCH));
        Ok(())
    }

    #[test]
    fn blender_like_control_modifier_suppresses_pinch() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        );
        route_to(&mut app, camera);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ControlLeft);
        app.world_mut().write_message(PinchGesture(2.0));

        app.update();

        let input = camera_input(&app, camera)?;
        assert_f32_close(input.zoom_smooth().amount(), 0.0);
        assert!(!input.sources().contains(InteractionSources::PINCH));
        Ok(())
    }

    #[test]
    fn non_routed_held_action_does_not_suppress_routed_pinch() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(OrbitCamPinchZoom)
            .build()
            .map_err(|_| "bindings should validate")?;
        let routed = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        route_to(&mut app, routed);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut().write_message(PinchGesture(2.0));

        app.update();

        let input = camera_input(&app, routed)?;
        assert_f32_close(
            input.zoom_smooth().amount(),
            2.0 * PINCH_GESTURE_AMPLIFICATION,
        );
        assert!(input.sources().contains(InteractionSources::PINCH));
        Ok(())
    }

    #[test]
    fn touch_adapter_resolves_to_orbit_input() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .touch(Some(OrbitCamTouchBinding::OneFingerOrbit))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        app.insert_resource(OrbitCamTouchAdapterOverride(TouchGestures::OneFinger(
            OneFingerGestures {
                motion: Vec2::new(7.0, 8.0),
            },
        )));

        app.update();

        let input = camera_input(&app, camera)?;
        assert_eq!(input.orbit(), OrbitDelta::from(Vec2::new(7.0, 8.0)));
        assert!(input.sources().contains(InteractionSources::TOUCH));
        Ok(())
    }

    #[test]
    fn touch_adapter_applies_per_action_input_gain() -> TestResult {
        let mut app = test_app();
        let input_gain = OrbitCamInputGain::new()
            .orbit(TOUCH_ORBIT_INPUT_GAIN)
            .pan(TOUCH_PAN_INPUT_GAIN)
            .zoom(TOUCH_ZOOM_INPUT_GAIN);
        let bindings = OrbitCamBindings::builder()
            .touch_config(Some(
                OrbitCamTouchBinding::OneFingerOrbit.with_input_gain(input_gain),
            ))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        app.insert_resource(OrbitCamTouchAdapterOverride(TouchGestures::TwoFinger(
            TwoFingerGestures {
                motion:   Vec2::new(TOUCH_MOTION_X, TOUCH_MOTION_Y),
                pinch:    TOUCH_PINCH_DELTA,
                rotation: 0.0,
            },
        )));

        app.update();

        let input = camera_input(&app, camera)?;
        assert_eq!(
            input.pan().pixels(),
            Vec2::new(TOUCH_MOTION_X, TOUCH_MOTION_Y) * TOUCH_PAN_INPUT_GAIN
        );
        assert_f32_close(
            input.zoom_smooth().amount(),
            TOUCH_PINCH_DELTA * TOUCH_PINCH_SCALE * TOUCH_ZOOM_INPUT_GAIN,
        );
        assert!(input.sources().contains(InteractionSources::TOUCH));
        Ok(())
    }

    #[test]
    fn zero_sensitive_touch_actions_are_inactive() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .touch_config(Some(
                OrbitCamTouchBinding::OneFingerOrbit
                    .with_input_gain(OrbitCamInputGain::uniform(DISABLED_INPUT_GAIN)),
            ))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        app.insert_resource(OrbitCamTouchAdapterOverride(TouchGestures::OneFinger(
            OneFingerGestures {
                motion: Vec2::new(TOUCH_MOTION_X, TOUCH_MOTION_Y),
            },
        )));

        app.update();
        assert_no_camera_input(&app, camera)?;

        app.insert_resource(OrbitCamTouchAdapterOverride(TouchGestures::TwoFinger(
            TwoFingerGestures {
                motion:   Vec2::new(TOUCH_MOTION_X, TOUCH_MOTION_Y),
                pinch:    TOUCH_PINCH_DELTA,
                rotation: 0.0,
            },
        )));
        app.update();

        assert_no_camera_input(&app, camera)
    }

    #[test]
    fn zero_sensitive_touch_action_does_not_disable_other_touch_actions() -> TestResult {
        let mut app = test_app();
        let input_gain = OrbitCamInputGain::new()
            .orbit(DISABLED_INPUT_GAIN)
            .pan(TOUCH_PAN_INPUT_GAIN)
            .zoom(DISABLED_INPUT_GAIN);
        let bindings = OrbitCamBindings::builder()
            .touch_config(Some(
                OrbitCamTouchBinding::OneFingerOrbit.with_input_gain(input_gain),
            ))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        app.insert_resource(OrbitCamTouchAdapterOverride(TouchGestures::OneFinger(
            OneFingerGestures {
                motion: Vec2::new(TOUCH_MOTION_X, TOUCH_MOTION_Y),
            },
        )));

        app.update();
        assert_no_camera_input(&app, camera)?;

        app.insert_resource(OrbitCamTouchAdapterOverride(TouchGestures::TwoFinger(
            TwoFingerGestures {
                motion:   Vec2::new(TOUCH_MOTION_X, TOUCH_MOTION_Y),
                pinch:    TOUCH_PINCH_DELTA,
                rotation: 0.0,
            },
        )));
        app.update();

        let input = camera_input(&app, camera)?;
        assert!(!input.has_orbit());
        assert_eq!(
            input.pan().pixels(),
            Vec2::new(TOUCH_MOTION_X, TOUCH_MOTION_Y) * TOUCH_PAN_INPUT_GAIN
        );
        assert!(!input.has_zoom());
        assert!(input.sources().contains(InteractionSources::TOUCH));
        Ok(())
    }

    #[test]
    fn button_drag_zoom_adapter_scales_and_reports_mouse() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(
                OrbitCamButtonDragZoom::new(MouseButton::Middle)
                    .with_axis(OrbitCamButtonDragZoomAxis::Y)
                    .with_input_gain(BUTTON_DRAG_INPUT_GAIN),
            )
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(0.0, BUTTON_DRAG_MOTION_Y);

        app.update();

        let input = camera_input(&app, camera)?;
        assert_f32_close(
            input.zoom_smooth().amount(),
            -BUTTON_DRAG_MOTION_Y * BUTTON_ZOOM_SCALE * BUTTON_DRAG_INPUT_GAIN,
        );
        assert!(input.sources().contains(InteractionSources::MOUSE));
        Ok(())
    }

    #[test]
    fn zero_sensitive_button_drag_zoom_is_inactive() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(
                OrbitCamButtonDragZoom::new(MouseButton::Middle)
                    .with_input_gain(DISABLED_INPUT_GAIN),
            )
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(0.0, BUTTON_DRAG_MOTION_Y);

        app.update();

        assert_no_camera_input(&app, camera)
    }

    #[test]
    fn keyboard_binding_resolves_to_smooth_zoom() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(HeldBinding::new(KeyCode::Equal, KeyCode::ShiftLeft))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        {
            let mut keyboard = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keyboard.press(KeyCode::Equal);
            keyboard.press(KeyCode::ShiftLeft);
        }

        app.update();

        let input = camera_input(&app, camera)?;
        assert_f32_close(input.zoom_smooth().amount(), 1.0);
        assert!(input.sources().contains(InteractionSources::KEYBOARD));
        Ok(())
    }

    #[test]
    fn gamepad_binding_resolves_to_orbit_input() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .orbit(HeldBinding::new(
                GamepadAxis::LeftStickX,
                GamepadButton::LeftTrigger2,
            ))
            .gamepad(CameraInputGamepadSelectionPolicy::Active)
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        let mut gamepad = Gamepad::default();
        gamepad.analog_mut().set(GamepadAxis::LeftStickX, 0.75);
        gamepad.analog_mut().set(GamepadButton::LeftTrigger2, 1.0);
        gamepad.digital_mut().press(GamepadButton::LeftTrigger2);
        app.world_mut().spawn(gamepad);

        app.update();

        let input = camera_input(&app, camera)?;
        assert_eq!(input.orbit().pixels(), Vec2::new(0.75, 0.0));
        assert!(input.sources().contains(InteractionSources::GAMEPAD));
        Ok(())
    }

    #[test]
    fn cardinal_keyboard_binding_resolves_to_orbit_input() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .orbit(InputBinding::cardinal_keys(
                KeyCode::ArrowUp,
                KeyCode::ArrowRight,
                KeyCode::ArrowDown,
                KeyCode::ArrowLeft,
            ))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        {
            let mut keyboard = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keyboard.press(KeyCode::ArrowRight);
            keyboard.press(KeyCode::ArrowUp);
        }

        app.update();

        let input = camera_input(&app, camera)?;
        assert_eq!(input.orbit().pixels(), Vec2::ONE);
        assert!(input.sources().contains(InteractionSources::KEYBOARD));
        Ok(())
    }

    #[test]
    fn bidirectional_keyboard_binding_resolves_to_smooth_zoom() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(InputBinding::bidirectional_keys(
                KeyCode::Equal,
                KeyCode::Minus,
            ))
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Minus);

        app.update();

        let input = camera_input(&app, camera)?;
        assert_f32_close(input.zoom_smooth().amount(), -1.0);
        assert!(input.sources().contains(InteractionSources::KEYBOARD));
        Ok(())
    }

    #[test]
    fn gamepad_axes2d_binding_resolves_to_orbit_input() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .orbit(InputBinding::gamepad_axes_2d(
                GamepadAxis::RightStickX,
                GamepadAxis::RightStickY,
            ))
            .gamepad(CameraInputGamepadSelectionPolicy::Active)
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        let mut gamepad = Gamepad::default();
        gamepad.analog_mut().set(GamepadAxis::RightStickX, 0.5);
        gamepad.analog_mut().set(GamepadAxis::RightStickY, -0.25);
        app.world_mut().spawn(gamepad);

        app.update();

        let input = camera_input(&app, camera)?;
        assert_eq!(input.orbit().pixels(), Vec2::new(0.5, -0.25));
        assert!(input.sources().contains(InteractionSources::GAMEPAD));
        Ok(())
    }

    #[test]
    fn bidirectional_gamepad_buttons_resolve_to_smooth_zoom() -> TestResult {
        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .zoom(InputBinding::bidirectional_gamepad_buttons(
                GamepadButton::RightTrigger2,
                GamepadButton::LeftTrigger2,
            ))
            .gamepad(CameraInputGamepadSelectionPolicy::Active)
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        let mut gamepad = Gamepad::default();
        gamepad.analog_mut().set(GamepadButton::LeftTrigger2, 0.4);
        gamepad.digital_mut().press(GamepadButton::LeftTrigger2);
        app.world_mut().spawn(gamepad);

        app.update();

        let input = camera_input(&app, camera)?;
        assert_f32_close(input.zoom_smooth().amount(), -0.4);
        assert!(input.sources().contains(InteractionSources::GAMEPAD));
        Ok(())
    }

    #[test]
    fn gamepad_preset_requires_no_position_route() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::gamepad()),
        );
        let mut gamepad = Gamepad::default();
        gamepad.analog_mut().set(GamepadAxis::RightStickX, 1.0);
        app.world_mut().spawn(gamepad);

        app.update();

        assert!(!camera_input(&app, camera)?.has_input());
        Ok(())
    }

    #[test]
    fn gamepad_preset_installs_modifiers_and_gate_conditions() {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::gamepad()),
        );
        route_to(&mut app, camera);

        app.update();

        let installed = input::installed_input_entities_for::<OrbitCamKind>(app.world(), camera);
        assert!(
            installed
                .iter()
                .any(|entity| app.world().get::<DeadZone>(*entity).is_some())
        );
        assert!(
            installed
                .iter()
                .any(|entity| app.world().get::<Scale>(*entity).is_some())
        );
        assert!(
            installed
                .iter()
                .any(|entity| app.world().get::<DeltaScale>(*entity).is_some())
        );
        assert!(installed.iter().any(|entity| {
            app.world()
                .get::<CameraBindingGateCondition>(*entity)
                .is_some()
        }));
    }

    /// Drives the gamepad preset's gated motion-split path with a real gamepad.
    /// The `InputReader` resolves a `GamepadButton` gate through its analog
    /// channel (`gamepad.get(button)`), so the gate button must have its analog
    /// value set — a bare `digital_mut().press` reads as `0.0` and never
    /// engages the gate. Right-stick orbit resolves `Normal`; adding the
    /// `RightTrigger` gate must flip the resolved speed to `Slow` while orbit
    /// stays active (fast is blocked by the same trigger, slow requires it).
    #[test]
    fn gamepad_preset_slow_orbit_gate_resolves_speed() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::gamepad()),
        );
        route_to(&mut app, camera);
        let gamepad_entity = app.world_mut().spawn(Gamepad::default()).id();
        app.update();

        {
            let mut gamepad = app
                .world_mut()
                .get_mut::<Gamepad>(gamepad_entity)
                .ok_or("gamepad should exist")?;
            gamepad.analog_mut().set(GamepadAxis::RightStickX, 1.0);
        }
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(16));
        app.update();
        assert!(camera_input(&app, camera)?.has_orbit());
        assert_eq!(
            camera_input(&app, camera)?.orbit_speed(),
            ControlSpeed::Normal
        );

        {
            let mut gamepad = app
                .world_mut()
                .get_mut::<Gamepad>(gamepad_entity)
                .ok_or("gamepad should exist")?;
            gamepad.analog_mut().set(GamepadButton::RightTrigger, 1.0);
            gamepad.digital_mut().press(GamepadButton::RightTrigger);
        }
        // The gate is a separate action the binding condition reads, so it
        // settles a frame after the trigger registers; keep the stick deflected
        // and advance time each frame so orbit stays active.
        for _ in 0..2 {
            {
                let mut gamepad = app
                    .world_mut()
                    .get_mut::<Gamepad>(gamepad_entity)
                    .ok_or("gamepad should exist")?;
                gamepad.analog_mut().set(GamepadAxis::RightStickX, 1.0);
            }
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(Duration::from_millis(16));
            app.update();
        }

        assert!(camera_input(&app, camera)?.has_orbit());
        assert_eq!(
            camera_input(&app, camera)?.orbit_speed(),
            ControlSpeed::Slow
        );
        Ok(())
    }

    /// Drives the gated motion-split path with a keyboard gate (gamepad digital
    /// buttons don't propagate through this harness). Fast orbit is blocked by
    /// Shift; the slow variant requires Shift and is tagged `Slow`, so pressing
    /// Shift must flip the resolved speed without changing the source set.
    #[test]
    fn keyboard_gated_slow_orbit_resolves_speed() -> TestResult {
        let bindings = OrbitCamBindings::builder()
            .orbit(
                HeldBinding::new(Binding::mouse_motion(), MouseButton::Left)
                    .with_blocked_gate(KeyCode::ShiftLeft),
            )
            .orbit(
                HeldBinding::new(Binding::mouse_motion(), MouseButton::Left)
                    .with_required_gate(KeyCode::ShiftLeft)
                    .speed(ControlSpeed::Slow),
            )
            .build()
            .map_err(|_| "bindings should build")?;

        let mut app = test_app();
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Bindings(bindings));
        route_to(&mut app, camera);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(4.0, -3.0);
        app.update();
        assert!(camera_input(&app, camera)?.has_orbit());
        assert_eq!(
            camera_input(&app, camera)?.orbit_speed(),
            ControlSpeed::Normal
        );

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        // The gate is a separate action the binding condition reads, so it
        // settles a frame after the key registers; re-assert motion each frame.
        for _ in 0..2 {
            app.world_mut()
                .resource_mut::<AccumulatedMouseMotion>()
                .delta = Vec2::new(4.0, -3.0);
            app.update();
        }
        assert!(camera_input(&app, camera)?.has_orbit());
        assert_eq!(
            camera_input(&app, camera)?.orbit_speed(),
            ControlSpeed::Slow
        );
        Ok(())
    }

    #[test]
    fn gamepad_gate_conditions_despawn_when_mode_changes() {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::gamepad()),
        );
        route_to(&mut app, camera);
        app.update();

        assert!(
            input::installed_input_entities_for::<OrbitCamKind>(app.world(), camera)
                .iter()
                .any(|entity| app
                    .world()
                    .get::<CameraBindingGateCondition>(*entity)
                    .is_some())
        );

        app.world_mut()
            .entity_mut(camera)
            .insert(OrbitCamInputMode::with_preset(
                OrbitCamPreset::simple_mouse(),
            ));
        app.update();

        assert!(
            !input::installed_input_entities_for::<OrbitCamKind>(app.world(), camera)
                .iter()
                .any(|entity| app
                    .world()
                    .get::<CameraBindingGateCondition>(*entity)
                    .is_some())
        );
    }

    #[test]
    fn invalid_direct_preset_replacement_preserves_installed_action_entities() -> TestResult {
        let mut app = test_app();
        let valid_mode = OrbitCamInputMode::with_preset(OrbitCamPreset::gamepad());
        let camera = spawn_camera(app.world_mut(), valid_mode.clone());
        route_to(&mut app, camera);
        app.update();

        let previous_entities =
            input::installed_input_entities_for::<OrbitCamKind>(app.world(), camera);
        let previous_actions = *app
            .world()
            .get::<OrbitCamInputActionEntities>(camera)
            .ok_or("camera should have input action entities")?;
        app.init_resource::<ModeReplacementEvents>();
        app.world_mut().entity_mut(camera).observe(
            |_: On<CameraInputModeReplaced>, mut events: ResMut<ModeReplacementEvents>| {
                events.0 += 1;
            },
        );

        app.world_mut()
            .entity_mut(camera)
            .insert(OrbitCamInputMode::with_preset(
                OrbitCamGamepadPreset::default()
                    .gamepad_input_gain(OrbitCamInputGain::uniform(INVALID_SOURCE_INPUT_GAIN)),
            ));
        app.update();

        assert_eq!(
            app.world().get::<OrbitCamInputMode>(camera),
            Some(&valid_mode)
        );
        assert_eq!(
            input::installed_input_entities_for::<OrbitCamKind>(app.world(), camera),
            previous_entities
        );
        assert_eq!(
            app.world().get::<OrbitCamInputActionEntities>(camera),
            Some(&previous_actions)
        );
        assert!(previous_entities.iter().any(|entity| {
            app.world()
                .get::<CameraBindingGateCondition>(*entity)
                .is_some()
        }));
        assert_eq!(app.world().resource::<ModeReplacementEvents>().0, 0);

        Ok(())
    }

    #[test]
    fn manual_mode_bypasses_action_resolution() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Manual);
        route_to(&mut app, camera);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Line,
            delta: Vec2::new(0.0, 3.0),
        };

        app.update();

        assert!(app.world().get::<OrbitCamInputContext>(camera).is_none());
        assert!(!camera_input(&app, camera)?.has_input());
        Ok(())
    }

    #[test]
    fn gated_camera_clears_previous_action_input() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        route_to(&mut app, camera);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Line,
            delta: Vec2::new(0.0, 3.0),
        };
        app.update();
        assert!(camera_input(&app, camera)?.has_zoom());

        app.world_mut()
            .entity_mut(camera)
            .insert(CameraInputDisabled);
        *app.world_mut().resource_mut::<AccumulatedMouseScroll>() = AccumulatedMouseScroll {
            unit:  MouseScrollUnit::Line,
            delta: Vec2::new(0.0, 3.0),
        };
        app.update();

        assert!(!camera_input(&app, camera)?.has_input());
        Ok(())
    }

    #[test]
    fn held_home_key_refires_once_when_input_reenables() -> TestResult {
        #[derive(Resource, Default)]
        struct CameraHomedCount(usize);

        let mut app = test_app();
        let bindings = OrbitCamBindings::builder()
            .home(KeyCode::KeyH)
            .build()
            .map_err(|_| "bindings should validate")?;
        let camera = spawn_camera(
            app.world_mut(),
            (
                OrbitCamInputMode::Bindings(bindings),
                CameraInputDisabled,
                OrbitCamHomePose {
                    orbit: OrbitAngles {
                        yaw:   HOME_ORBIT_YAW,
                        pitch: HOME_ORBIT_PITCH,
                    },
                    pan:   Focus(HOME_FOCUS),
                    zoom:  Radius(HOME_ZOOM_RADIUS),
                },
            ),
        );
        route_to(&mut app, camera);
        app.init_resource::<CameraHomedCount>();
        app.add_observer(|_: On<CameraHomed>, mut count: ResMut<CameraHomedCount>| {
            count.0 += 1;
        });
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyH);

        app.update();
        assert_eq!(app.world().resource::<CameraHomedCount>().0, 0);

        app.world_mut()
            .entity_mut(camera)
            .remove::<CameraInputDisabled>();
        app.update();
        app.update();

        assert_eq!(app.world().resource::<CameraHomedCount>().0, 1);
        Ok(())
    }
}
