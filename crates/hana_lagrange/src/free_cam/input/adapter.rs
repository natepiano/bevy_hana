use bevy::ecs::system::SystemParam;
use bevy::input::gamepad::Gamepad;
use bevy::prelude::App;
use bevy::prelude::ButtonInput;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::MouseButton;
use bevy::prelude::Plugin;
use bevy::prelude::PreUpdate;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy_enhanced_input::prelude::Accumulation;
use bevy_enhanced_input::prelude::Action;
use bevy_enhanced_input::prelude::ActionSettings;
use bevy_enhanced_input::prelude::Actions;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::ContextActivity;
use bevy_enhanced_input::prelude::ContextPriority;
use bevy_enhanced_input::prelude::GamepadDevice;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::TriggerState;

use super::FreeCamBindings;
use super::FreeCamLookPitch;
use crate::FreeCam;
use crate::FreeCamHomePose;
use crate::FreeCamKind;
use crate::camera_home::CameraHomeResetSources;
use crate::input;
use crate::input::BindingGates;
use crate::input::CameraActionResolutionContext;
use crate::input::CameraActionResolutionKind;
use crate::input::CameraInputGamepadSelectionPolicy;
use crate::input::CameraInstallKind;
use crate::input::CameraInstalledBindings;
use crate::input::CameraSlowMode;
use crate::input::ControlSpeed;
use crate::input::FreeCamActiveDirections;
use crate::input::FreeCamControlDirection;
use crate::input::FreeCamGateAction;
use crate::input::FreeCamHomeAction;
use crate::input::FreeCamInputContext;
use crate::input::FreeCamLookAction;
use crate::input::FreeCamLookButtonAction;
use crate::input::FreeCamResolvedBindings;
use crate::input::FreeCamRollAction;
use crate::input::FreeCamRollEngagedAction;
use crate::input::FreeCamSlowModeToggleAction;
use crate::input::FreeCamTranslateAction;
use crate::input::FreeCamTranslateEngagedAction;
use crate::input::GateActionCache;
use crate::input::GateInput;
use crate::input::GatePolarity;
use crate::input::HeldActionBindingEntry;
use crate::input::HeldCameraAction;
use crate::input::InteractionSources;
use crate::input::LiveInputs;
use crate::input::MotionActions;
use crate::input::NoActionFrameState;
use crate::input::ResetFreeCamToHome;
use crate::system_sets::CameraInputInternalSet;

/// Installs and resolves `FreeCam` preset/binding actions.
pub struct FreeCamInputAdapterPlugin;

impl Plugin for FreeCamInputAdapterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                clear_replaced_or_manual_installations,
                install_enhanced_input_entities,
            )
                .chain()
                .in_set(CameraInputInternalSet::Installation),
        )
        .add_systems(
            PreUpdate,
            (
                input::resolve_actions_into_camera_input::<FreeCamKind>,
                apply_free_cam_home,
            )
                .chain()
                .in_set(CameraInputInternalSet::ActionResolution),
        );
    }
}

impl CameraInstallKind for FreeCamKind {
    type GateAction = FreeCamGateAction;
}

/// Whether the home action was active last frame, so `apply_free_cam_home` emits
/// [`CameraHomed`] only on the rising edge rather than every held frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HomeActionState {
    Active,
    Inactive,
}

impl From<bool> for HomeActionState {
    fn from(active: bool) -> Self { if active { Self::Active } else { Self::Inactive } }
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FreeCamInputActionEntities {
    translate:            Entity,
    translate_engaged:    Entity,
    look:                 Entity,
    look_button:          Entity,
    roll:                 Entity,
    roll_engaged:         Entity,
    slow_toggle:          Entity,
    home:                 Entity,
    translate_boost_gate: Option<Entity>,
    translate_sources:    InteractionSources,
    look_sources:         InteractionSources,
    roll_sources:         InteractionSources,
    home_sources:         InteractionSources,
    home_state:           HomeActionState,
}

fn clear_replaced_or_manual_installations(world: &mut World) {
    let mut query = world.query_filtered::<Entity, With<FreeCamInputContext>>();
    let cameras = query.iter(world).collect::<Vec<_>>();

    for camera in cameras {
        let installed_entities = input::installed_input_entities_for::<FreeCamKind>(world, camera);
        if installed_entities.is_empty()
            || input::input_installation_has_placeholder_for::<FreeCamKind>(world, camera)
        {
            clear_enhanced_input_components(world, camera);
        }
    }
}

fn clear_enhanced_input_components(world: &mut World, camera: Entity) {
    let mut entity = world.entity_mut(camera);
    entity
        .remove::<FreeCamInputContext>()
        .remove::<ContextActivity<FreeCamInputContext>>()
        .remove::<ContextPriority<FreeCamInputContext>>()
        .remove::<GamepadDevice>()
        .remove::<Actions<FreeCamInputContext>>()
        .remove::<FreeCamInputActionEntities>()
        .remove::<CameraInstalledBindings<FreeCamKind>>();
}

const fn gamepad_device_for(bindings: &FreeCamBindings) -> GamepadDevice {
    match bindings.gamepad() {
        CameraInputGamepadSelectionPolicy::Disabled => GamepadDevice::None,
        CameraInputGamepadSelectionPolicy::Active => GamepadDevice::Any,
    }
}

fn install_enhanced_input_entities(world: &mut World) {
    let mut query = world.query_filtered::<(Entity, &FreeCamResolvedBindings), With<FreeCam>>();
    let cameras = query
        .iter(world)
        .filter(|(camera, _)| {
            input::input_installation_has_placeholder_for::<FreeCamKind>(world, *camera)
        })
        .map(|(camera, bindings)| (camera, bindings.0.clone()))
        .collect::<Vec<_>>();

    for (camera, bindings) in cameras {
        for installed_entity in input::installed_input_entities_for::<FreeCamKind>(world, camera) {
            let _ = world.despawn(installed_entity);
        }

        let installation = spawn_input_installation(world, camera, &bindings);
        world.entity_mut(camera).insert((
            FreeCamInputContext,
            gamepad_device_for(&bindings),
            CameraInstalledBindings::<FreeCamKind>(bindings),
            installation.actions,
        ));
        input::replace_installed_input_entities_for::<FreeCamKind>(
            world,
            camera,
            installation.entities,
        );
    }
}

struct SpawnedInputInstallation {
    actions:  FreeCamInputActionEntities,
    entities: Vec<Entity>,
}

fn spawn_input_installation(
    world: &mut World,
    camera: Entity,
    bindings: &FreeCamBindings,
) -> SpawnedInputInstallation {
    let translate = input::spawn_action::<FreeCamTranslateAction, FreeCamKind>(world, camera);
    let translate_engaged = spawn_engagement_action::<FreeCamTranslateEngagedAction>(world, camera);
    let look = input::spawn_action::<FreeCamLookAction, FreeCamKind>(world, camera);
    let look_button = spawn_engagement_action::<FreeCamLookButtonAction>(world, camera);
    let roll = input::spawn_action::<FreeCamRollAction, FreeCamKind>(world, camera);
    let roll_engaged = spawn_engagement_action::<FreeCamRollEngagedAction>(world, camera);
    let slow_toggle =
        input::spawn_action::<FreeCamSlowModeToggleAction, FreeCamKind>(world, camera);
    let home = input::spawn_action::<FreeCamHomeAction, FreeCamKind>(world, camera);

    let mut entities = vec![
        translate,
        translate_engaged,
        look,
        look_button,
        roll,
        roll_engaged,
        slow_toggle,
        home,
    ];
    let mut gate_actions = GateActionCache::default();

    input::spawn_held_bindings::<_, FreeCamKind>(
        world,
        camera,
        MotionActions {
            normal: translate,
            slow:   translate,
        },
        translate_engaged,
        bindings.translate().enabled_entries(),
        &mut gate_actions,
        &mut entities,
    );
    input::spawn_held_bindings::<_, FreeCamKind>(
        world,
        camera,
        MotionActions {
            normal: look,
            slow:   look,
        },
        look_button,
        bindings.look().enabled_entries(),
        &mut gate_actions,
        &mut entities,
    );
    input::spawn_held_bindings::<_, FreeCamKind>(
        world,
        camera,
        MotionActions {
            normal: roll,
            slow:   roll,
        },
        roll_engaged,
        bindings.roll().enabled_entries(),
        &mut gate_actions,
        &mut entities,
    );
    if let Some(slow_mode) = bindings.slow_mode() {
        entities.push(input::spawn_single_binding::<FreeCamKind>(
            world,
            slow_toggle,
            camera,
            Binding::Keyboard {
                key:      slow_mode.toggle_key,
                mod_keys: slow_mode.mod_keys,
            },
        ));
    }
    for entry in bindings.enabled_home_entries() {
        input::spawn_binding::<FreeCamKind>(
            world,
            camera,
            home,
            entry.binding_descriptor(),
            &BindingGates::default(),
            &mut gate_actions,
            &mut entities,
        );
    }

    let translate_boost_gate = translate_boost_gate_input(bindings.translate().enabled_entries())
        .and_then(|input| gate_actions.action_for(input));

    let actions = FreeCamInputActionEntities {
        translate,
        translate_engaged,
        look,
        look_button,
        roll,
        roll_engaged,
        slow_toggle,
        home,
        translate_boost_gate,
        translate_sources: input::held_sources(bindings.translate().enabled_entries()),
        look_sources: input::held_sources(bindings.look().enabled_entries()),
        roll_sources: input::held_sources(bindings.roll().enabled_entries()),
        home_sources: input::action_sources(bindings.enabled_home_entries()),
        home_state: HomeActionState::Inactive,
    };

    SpawnedInputInstallation { actions, entities }
}

/// Spawns an engagement gate action that reports "any constituent input is active".
///
/// A held binding's engagement descriptor carries the motion's axis transforms, so an
/// opposing key pair (`Q`/`E`, `W`/`S`) would sum to zero under the default
/// [`Accumulation::Cumulative`] and read as disengaged. [`Accumulation::MaxAbs`] takes the
/// strongest per-axis input instead, so the gate stays engaged whenever any key or axis is held.
fn spawn_engagement_action<A: InputAction>(world: &mut World, camera: Entity) -> Entity {
    let action = input::spawn_action::<A, FreeCamKind>(world, camera);
    world.entity_mut(action).insert(ActionSettings {
        accumulation:  Accumulation::MaxAbs,
        require_reset: false,
        consume_input: false,
    });
    action
}

impl CameraActionResolutionKind for FreeCamKind {
    type ActionEntities = FreeCamInputActionEntities;
    type FrameState = NoActionFrameState;
    type ActionQueries<'w, 's> = FreeCamActionQueries<'w, 's>;

    fn slow_mode_toggle_action(actions: &Self::ActionEntities) -> Option<Entity> {
        Some(actions.slow_toggle)
    }

    fn resolve_camera_actions(
        context: CameraActionResolutionContext<'_, Self>,
        action_queries: &Self::ActionQueries<'_, '_>,
        states: &Query<&TriggerState>,
        _: &LiveInputs<'_>,
    ) {
        let input_speed = input_speed(context.bindings.0.slow_mode(), context.slow_mode_active);
        let scale = scale_factor(&context.bindings.0, context.slow_mode_active);
        let look_pitch = context.bindings.0.look_pitch();
        let actions = context.actions;
        let input = context.input;
        let translate_active = bool_action_active(
            actions.translate_engaged,
            &action_queries.translate_engaged,
            states,
        );
        let look_active =
            bool_action_active(actions.look_button, &action_queries.look_button, states);
        let roll_active =
            bool_action_active(actions.roll_engaged, &action_queries.roll_engaged, states);
        let boost = BoostGate::from(actions.translate_boost_gate.is_some_and(|gate| {
            bool_action_active(gate, &action_queries.translate_boost_gate, states)
        }));
        let mut directions = FreeCamActiveDirections::NONE;
        if translate_active {
            let translation = action_value(actions.translate, &action_queries.translate);
            input.add_translate_with_sources(translation * scale, actions.translate_sources);
            input.set_translate_speed(input_speed);
            directions = translate_directions(directions, translation, boost);
        }
        if look_active {
            let look = pitched_look(action_value(actions.look, &action_queries.look), look_pitch);
            input.add_look_with_sources(look * scale, actions.look_sources);
            input.set_look_speed(input_speed);
        }
        if roll_active {
            let roll = action_value(actions.roll, &action_queries.roll);
            input.add_roll_with_sources(roll * scale, actions.roll_sources);
            input.set_roll_speed(input_speed);
            directions = roll_directions(directions, roll);
        }
        input.set_directions(directions);
    }
}

/// Whether the move-speed boost gate is held, deciding whether horizontal stick
/// input reports as [`FreeCamControlDirection::Boost`] or
/// [`FreeCamControlDirection::Stick`].
#[derive(Clone, Copy)]
enum BoostGate {
    Engaged,
    Disengaged,
}

impl From<bool> for BoostGate {
    fn from(engaged: bool) -> Self {
        if engaged {
            Self::Engaged
        } else {
            Self::Disengaged
        }
    }
}

impl BoostGate {
    const fn horizontal(self) -> FreeCamControlDirection {
        match self {
            Self::Engaged => FreeCamControlDirection::Boost,
            Self::Disengaged => FreeCamControlDirection::Stick,
        }
    }
}

/// Adds the horizontal and vertical directions the resolved gamepad move vector
/// engages: any strafe/forward component lights the stick (or boost) row, and the
/// vertical sign lights the up or down trigger row.
fn translate_directions(
    directions: FreeCamActiveDirections,
    translation: Vec3,
    boost: BoostGate,
) -> FreeCamActiveDirections {
    let mut directions = directions;
    if translation.x != 0.0 || translation.z != 0.0 {
        directions = directions.with(boost.horizontal());
    }
    if translation.y > 0.0 {
        directions = directions.with(FreeCamControlDirection::Up);
    } else if translation.y < 0.0 {
        directions = directions.with(FreeCamControlDirection::Down);
    }
    directions
}

/// Adds the roll direction the resolved roll sign engages.
fn roll_directions(directions: FreeCamActiveDirections, roll: f32) -> FreeCamActiveDirections {
    if roll > 0.0 {
        directions.with(FreeCamControlDirection::RollRight)
    } else if roll < 0.0 {
        directions.with(FreeCamControlDirection::RollLeft)
    } else {
        directions
    }
}

/// Returns the gate input of the first `Required`-gated translate entry — the
/// move-speed boost gate for the gamepad preset — so its gate action can report
/// whether boost is engaged.
fn translate_boost_gate_input<'a, A: HeldCameraAction + 'a>(
    entries: impl Iterator<Item = &'a HeldActionBindingEntry<A>>,
) -> Option<GateInput> {
    for entry in entries {
        for gate in entry.gates().entries() {
            if gate.polarity == GatePolarity::Required {
                return Some(gate.input);
            }
        }
    }
    None
}

const fn pitched_look(look: Vec2, look_pitch: FreeCamLookPitch) -> Vec2 {
    match look_pitch {
        FreeCamLookPitch::Normal => look,
        FreeCamLookPitch::Inverted => Vec2::new(look.x, -look.y),
    }
}

const fn input_speed(slow_mode: Option<&CameraSlowMode>, slow_mode_active: bool) -> ControlSpeed {
    if slow_mode.is_some() && slow_mode_active {
        ControlSpeed::Slow
    } else {
        ControlSpeed::Normal
    }
}

fn scale_factor(bindings: &FreeCamBindings, slow_mode_active: bool) -> f32 {
    bindings.slow_mode().map_or(1.0, |slow_mode| {
        if slow_mode_active {
            slow_mode.scale.slow
        } else {
            slow_mode.scale.normal
        }
    })
}

/// Triggers a `FreeCam` home reset on the home action's rising edge.
///
/// The reset observer performs the retarget and emits [`crate::input::CameraHomed`]
/// with the attributed sources stored by this system.
fn apply_free_cam_home(
    mut commands: Commands,
    mut cameras: Query<
        (
            Entity,
            &mut FreeCamInputActionEntities,
            &CameraInstalledBindings<FreeCamKind>,
        ),
        (With<FreeCam>, With<FreeCamHomePose>),
    >,
    home_actions: Query<&Action<FreeCamHomeAction>>,
    states: Query<&TriggerState>,
    keyboard: Option<Res<ButtonInput<KeyCode>>>,
    mouse_buttons: Option<Res<ButtonInput<MouseButton>>>,
    gamepads: Query<&Gamepad>,
) {
    let gamepad_refs = gamepads.iter().collect::<Vec<_>>();
    let inputs = LiveInputs {
        keyboard: keyboard.as_deref(),
        mouse:    mouse_buttons.as_deref(),
        gamepads: gamepad_refs.as_slice(),
    };

    for (camera, mut actions, bindings) in &mut cameras {
        let active = bool_action_active(actions.home, &home_actions, &states);
        let next = HomeActionState::from(active);
        if actions.home_state != next {
            if matches!(next, HomeActionState::Active) {
                let sources = input::attributed_sources(
                    bindings.0.enabled_home_entries(),
                    &inputs,
                    actions.home_sources,
                );
                commands
                    .entity(camera)
                    .insert(CameraHomeResetSources(sources));
                commands.trigger(ResetFreeCamToHome { camera });
            }
            actions.home_state = next;
        }
    }
}

#[derive(SystemParam)]
pub(crate) struct FreeCamActionQueries<'w, 's> {
    translate:            Query<'w, 's, &'static Action<FreeCamTranslateAction>>,
    translate_engaged:    Query<'w, 's, &'static Action<FreeCamTranslateEngagedAction>>,
    translate_boost_gate: Query<'w, 's, &'static Action<FreeCamGateAction>>,
    look:                 Query<'w, 's, &'static Action<FreeCamLookAction>>,
    look_button:          Query<'w, 's, &'static Action<FreeCamLookButtonAction>>,
    roll:                 Query<'w, 's, &'static Action<FreeCamRollAction>>,
    roll_engaged:         Query<'w, 's, &'static Action<FreeCamRollEngagedAction>>,
}

fn bool_action_active<A: InputAction<Output = bool>>(
    action: Entity,
    actions: &Query<&Action<A>>,
    states: &Query<&TriggerState>,
) -> bool {
    action_state_active(action, states) && actions.get(action).is_ok_and(|action| **action)
}

fn action_state_active(action: Entity, states: &Query<&TriggerState>) -> bool {
    states
        .get(action)
        .is_ok_and(|state| matches!(*state, TriggerState::Ongoing | TriggerState::Fired))
}

fn action_value<A: InputAction>(action: Entity, actions: &Query<&Action<A>>) -> A::Output {
    actions
        .get(action)
        .map(|action| **action)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use bevy::camera::RenderTarget;
    use bevy::input::mouse::AccumulatedMouseMotion;
    use bevy::prelude::Camera;
    use bevy::prelude::GamepadAxis;
    use bevy::prelude::GamepadButton;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::window::WindowRef;
    use bevy_enhanced_input::prelude::EnhancedInputPlugin;
    use bevy_enhanced_input::prelude::InputContextAppExt;

    use super::*;
    use crate::FreeCamPreset;
    use crate::free_cam::FreeCamInput;
    use crate::input::CameraHomed;
    use crate::input::CameraInputModesPlugin;
    use crate::input::CameraInputRoutingConfig;
    use crate::input::CameraInputRoutingPlugin;
    use crate::input::FreeCamGamepadPreset;
    use crate::input::FreeCamInputGain;
    use crate::input::FreeCamInputMode;
    use crate::input::FreeCamKeyboardMousePreset;
    use crate::input::FreeCamLookPitch;
    use crate::operation::LookAngles;
    use crate::operation::Position;
    use crate::operation::Roll;
    use crate::system_sets::LagrangeSystemSetsPlugin;

    const RUNTIME_LOOK_INPUT_GAIN: f32 = 0.25;
    const RUNTIME_MOUSE_DELTA: Vec2 = Vec2::new(2.0, 4.0);
    const RUNTIME_ROLL_INPUT_GAIN: f32 = 0.75;
    const RUNTIME_TRANSLATE_INPUT_GAIN: f32 = 0.5;
    /// Looser than `f32::EPSILON`: the adapter bakes gain into the binding scale
    /// at install, so the resolved value is `(scale · gain)` while the test
    /// recomputes `(baseline · gain)`; float multiplication is not associative.
    const SCALE_COMPOSITION_TOLERANCE: f32 = 1e-6;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            EnhancedInputPlugin,
            LagrangeSystemSetsPlugin,
            CameraInputModesPlugin,
            CameraInputRoutingPlugin,
            FreeCamInputAdapterPlugin,
        ))
        .add_input_context::<FreeCamInputContext>();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>();
        crate::camera_home::add_free_cam_home_reset_systems(&mut app);
        app.finish();
        app
    }

    fn spawn_camera(app: &mut App) -> Entity {
        spawn_camera_with_mode(
            app,
            FreeCamInputMode::with_preset(FreeCamPreset::keyboard_mouse()),
        )
    }

    fn spawn_camera_with_mode(app: &mut App, free_cam_input_mode: FreeCamInputMode) -> Entity {
        app.world_mut()
            .spawn((
                FreeCam::default(),
                FreeCamInput::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                free_cam_input_mode,
            ))
            .id()
    }

    type TestResult = Result<(), &'static str>;

    #[test]
    fn keyboard_bindings_resolve_translate_and_roll() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(&mut app);
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyQ);
        app.update();

        let input = app
            .world()
            .get::<FreeCamInput>(camera)
            .ok_or("camera should have input")?;
        assert_eq!(input.translate().vector(), Vec3::NEG_Z);
        assert_eq!(input.translate_sources(), InteractionSources::KEYBOARD);
        assert_eq!(input.translate_speed(), ControlSpeed::Normal);
        assert!((input.roll().amount() - 1.0).abs() <= f32::EPSILON);
        assert_eq!(input.roll_sources(), InteractionSources::KEYBOARD);
        assert_eq!(input.roll_speed(), ControlSpeed::Normal);
        Ok(())
    }

    #[test]
    fn cancelling_keyboard_bindings_still_report_active_sources() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(&mut app);
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.update();

        for key in [KeyCode::KeyW, KeyCode::KeyS, KeyCode::KeyQ, KeyCode::KeyE] {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(key);
        }
        app.update();

        let input = app
            .world()
            .get::<FreeCamInput>(camera)
            .ok_or("camera should have input")?;
        assert_eq!(input.translate().vector(), Vec3::ZERO);
        assert!(input.has_translate());
        assert_eq!(input.translate_sources(), InteractionSources::KEYBOARD);
        assert_eq!(input.translate_speed(), ControlSpeed::Normal);
        assert!(input.roll().amount().abs() <= f32::EPSILON);
        assert!(input.has_roll());
        assert_eq!(input.roll_sources(), InteractionSources::KEYBOARD);
        assert_eq!(input.roll_speed(), ControlSpeed::Normal);
        Ok(())
    }

    #[test]
    fn held_keyboard_bindings_continue_reporting_sources() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(&mut app);
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyQ);
        app.update();
        app.update();

        let input = app
            .world()
            .get::<FreeCamInput>(camera)
            .ok_or("camera should have input")?;
        assert_eq!(input.translate_sources(), InteractionSources::KEYBOARD);
        assert_eq!(input.translate_speed(), ControlSpeed::Normal);
        assert_eq!(input.roll_sources(), InteractionSources::KEYBOARD);
        assert_eq!(input.roll_speed(), ControlSpeed::Normal);
        Ok(())
    }

    #[test]
    fn mouse_motion_requires_look_button() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(&mut app);
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.update();

        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(2.0, 3.0);
        app.update();
        let input = app
            .world()
            .get::<FreeCamInput>(camera)
            .ok_or("camera should have input")?;
        assert!(!input.has_look());

        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.update();

        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(2.0, 3.0);
        app.update();

        let input = app
            .world()
            .get::<FreeCamInput>(camera)
            .ok_or("camera should have input")?;
        assert_eq!(input.look().pixels(), Vec2::new(2.0, 3.0));
        Ok(())
    }

    #[test]
    fn inverted_pitch_negates_mouse_y() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera_with_mode(
            &mut app,
            FreeCamInputMode::with_preset(
                FreeCamKeyboardMousePreset::default().with_look_pitch(FreeCamLookPitch::Inverted),
            ),
        );
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.update();

        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(2.0, 3.0);
        app.update();

        let input = app
            .world()
            .get::<FreeCamInput>(camera)
            .ok_or("camera should have input")?;
        assert_eq!(input.look().pixels(), Vec2::new(2.0, -3.0));
        Ok(())
    }

    #[test]
    fn keyboard_mouse_source_input_gain_scales_resolved_input() -> TestResult {
        let mut app = test_app();
        let input_gain = FreeCamInputGain::new()
            .translate(RUNTIME_TRANSLATE_INPUT_GAIN)
            .look(RUNTIME_LOOK_INPUT_GAIN);
        let camera = spawn_camera_with_mode(
            &mut app,
            FreeCamInputMode::with_preset(
                FreeCamKeyboardMousePreset::default().mouse_input_gain(input_gain),
            ),
        );
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.update();

        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = RUNTIME_MOUSE_DELTA;
        app.update();

        let input = app
            .world()
            .get::<FreeCamInput>(camera)
            .ok_or("camera should have input")?;
        assert_eq!(
            input.translate().vector(),
            Vec3::NEG_Z * RUNTIME_TRANSLATE_INPUT_GAIN
        );
        assert_eq!(
            input.look().pixels(),
            RUNTIME_MOUSE_DELTA * RUNTIME_LOOK_INPUT_GAIN
        );
        Ok(())
    }

    /// Resolves one frame of stick-plus-trigger gamepad input for `preset` and
    /// returns the translate vector and roll amount it produced.
    fn resolved_gamepad_input(preset: FreeCamGamepadPreset) -> Result<(Vec3, f32), &'static str> {
        let mut app = test_app();
        let camera = spawn_camera_with_mode(&mut app, FreeCamInputMode::with_preset(preset));
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.update();

        let mut gamepad = Gamepad::default();
        gamepad.analog_mut().set(GamepadAxis::LeftStickX, 0.75);
        gamepad.analog_mut().set(GamepadButton::RightTrigger, 1.0);
        gamepad.digital_mut().press(GamepadButton::RightTrigger);
        app.world_mut().spawn(gamepad);
        app.update();

        let input = app
            .world()
            .get::<FreeCamInput>(camera)
            .ok_or("camera should have input")?;
        Ok((input.translate().vector(), input.roll().amount()))
    }

    #[test]
    fn gamepad_source_input_gain_scales_resolved_input() -> TestResult {
        let (baseline_translate, baseline_roll) =
            resolved_gamepad_input(FreeCamGamepadPreset::default())?;
        assert_ne!(baseline_translate, Vec3::ZERO);
        assert!(baseline_roll.abs() > f32::EPSILON);

        let input_gain = FreeCamInputGain::new()
            .translate(RUNTIME_TRANSLATE_INPUT_GAIN)
            .roll(RUNTIME_ROLL_INPUT_GAIN);
        let (tuned_translate, tuned_roll) =
            resolved_gamepad_input(FreeCamGamepadPreset::default().gamepad_input_gain(input_gain))?;

        let expected_translate = baseline_translate * RUNTIME_TRANSLATE_INPUT_GAIN;
        assert!(
            (tuned_translate - expected_translate).abs().max_element()
                <= SCALE_COMPOSITION_TOLERANCE
        );
        assert!(
            baseline_roll
                .mul_add(-RUNTIME_ROLL_INPUT_GAIN, tuned_roll)
                .abs()
                <= SCALE_COMPOSITION_TOLERANCE
        );
        Ok(())
    }

    #[test]
    fn home_action_resets_targets_to_home_pose() -> TestResult {
        let mut app = test_app();
        let bindings = FreeCamBindings::builder()
            .home(KeyCode::KeyH)
            .build()
            .map_err(|_| "bindings should build")?;
        let camera = spawn_camera_with_mode(&mut app, FreeCamInputMode::Bindings(bindings));

        let home_pose = FreeCamHomePose {
            position: Position(Vec3::new(1.0, 2.0, 3.0)),
            look:     LookAngles {
                yaw:   0.5,
                pitch: 0.25,
            },
            roll:     Roll(0.1),
        };
        app.world_mut().entity_mut(camera).insert(home_pose);
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.update();

        // Drive the eased targets away from home, then press the home key.
        {
            let mut free_cam = app
                .world_mut()
                .get_mut::<FreeCam>(camera)
                .ok_or("camera should have FreeCam")?;
            free_cam
                .translate
                .set_target(Position(Vec3::new(9.0, 9.0, 9.0)));
            free_cam.look.set_target(LookAngles {
                yaw:   2.0,
                pitch: 1.0,
            });
            free_cam.roll.set_target(Roll(1.0));
        }

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyH);
        app.update();

        let free_cam = app
            .world()
            .get::<FreeCam>(camera)
            .ok_or("camera should have FreeCam")?;
        assert_eq!(free_cam.translate.target(), home_pose.position);
        assert_eq!(free_cam.look.target(), home_pose.look);
        assert_eq!(free_cam.roll.target(), home_pose.roll);
        Ok(())
    }

    #[test]
    fn home_action_emits_camera_homed_on_rising_edge() -> TestResult {
        #[derive(Resource, Default)]
        struct CameraHomedCount(usize);

        let mut app = test_app();
        let bindings = FreeCamBindings::builder()
            .home(KeyCode::KeyH)
            .build()
            .map_err(|_| "bindings should build")?;
        let camera = spawn_camera_with_mode(&mut app, FreeCamInputMode::Bindings(bindings));
        app.world_mut().entity_mut(camera).insert(FreeCamHomePose {
            position: Position(Vec3::ZERO),
            look:     LookAngles {
                yaw:   0.0,
                pitch: 0.0,
            },
            roll:     Roll(0.0),
        });
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.init_resource::<CameraHomedCount>();
        app.add_observer(|_: On<CameraHomed>, mut count: ResMut<CameraHomedCount>| {
            count.0 += 1;
        });
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyH);
        app.update();

        assert_eq!(app.world().resource::<CameraHomedCount>().0, 1);
        Ok(())
    }
}
