//! Installation, teardown, and context-gating for the enhanced-input adapter pipeline.
//!
//! Shared adapter carrier types live in `adapter/mod.rs`; this module installs
//! [`CameraInstalledBindings<OrbitCamKind>`] and [`OrbitCamInputActionEntities`] on each camera.
//!
//! Types:
//! - [`OrbitCamAdapterDiagnostics`] — resource updated each frame with counts of installed cameras
//!   / installed entities / gated cameras.
//! - [`SpawnedInputInstallation`] — file-local return type of [`spawn_input_installation`].
//!
//! Systems (registered in [`CameraInputInternalSet::Installation`]):
//! - [`clear_replaced_or_manual_installations`] — strips enhanced-input components when the
//!   camera's input mode was replaced or has a placeholder marker.
//! - [`install_enhanced_input_entities`] — for each camera with a placeholder, builds the
//!   `OrbitCamBindings`, spawns the action entities, delegates binding and gate installation to the
//!   shared install driver, and inserts the components in this file.
//! - [`apply_context_gating`] — flips each camera's [`ContextActivity<OrbitCamInputContext>`] based
//!   on its [`CameraInputContextGated`] state.

use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Resource;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy_enhanced_input::prelude::ActionValue;
use bevy_enhanced_input::prelude::Actions;
use bevy_enhanced_input::prelude::ActionsQuery;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::ContextActivity;
use bevy_enhanced_input::prelude::ContextPriority;
use bevy_enhanced_input::prelude::ContextTime;
use bevy_enhanced_input::prelude::CustomInput;
use bevy_enhanced_input::prelude::CustomInputs;
use bevy_enhanced_input::prelude::GamepadDevice;
use bevy_enhanced_input::prelude::InputCondition;
use bevy_enhanced_input::prelude::ModKeys;
use bevy_enhanced_input::prelude::Negate;
use bevy_enhanced_input::prelude::Press;
use bevy_enhanced_input::prelude::Scale;
use bevy_enhanced_input::prelude::SwizzleAxis;
use bevy_enhanced_input::prelude::TriggerState;

use super::HomeActionState;
use super::OrbitCamAdapterFrameSources;
use super::OrbitCamInputActionEntities;
use super::inject::TrackpadScrollTarget;
use crate::OrbitCamKind;
use crate::input;
use crate::input::BindingGates;
use crate::input::CameraInputContextGated;
use crate::input::CameraInputGamepadSelectionPolicy;
use crate::input::CameraInstallKind;
use crate::input::CameraInstalledBindings;
use crate::input::GateActionCache;
use crate::input::MotionActions;
use crate::input::OrbitCamAdapterOrbitAction;
use crate::input::OrbitCamAdapterPanAction;
use crate::input::OrbitCamAdapterZoomCoarseAction;
use crate::input::OrbitCamAdapterZoomSmoothAction;
use crate::input::OrbitCamBindingWithInputGain;
use crate::input::OrbitCamBindings;
use crate::input::OrbitCamGateAction;
use crate::input::OrbitCamHomeAction;
use crate::input::OrbitCamInputContext;
use crate::input::OrbitCamOrbitAction;
use crate::input::OrbitCamOrbitEngagedAction;
use crate::input::OrbitCamOrbitSlowAction;
use crate::input::OrbitCamPanAction;
use crate::input::OrbitCamPanEngagedAction;
use crate::input::OrbitCamPanSlowAction;
use crate::input::OrbitCamResolvedBindings;
use crate::input::OrbitCamSlowModeToggleAction;
use crate::input::OrbitCamTrackpadScroll;
use crate::input::OrbitCamZoomCoarseAction;
use crate::input::OrbitCamZoomEngagedAction;
use crate::input::OrbitCamZoomSmoothAction;
use crate::input::OrbitCamZoomSmoothSlowAction;
use crate::input::PIXEL_SCROLL_SCALE;
use crate::input::ZoomInversion;
use crate::orbit_cam::OrbitCam;

impl CameraInstallKind for OrbitCamKind {
    type GateAction = OrbitCamGateAction;
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct OrbitCamAdapterCustomInputs {
    pub(super) orbit:       CustomInput,
    pub(super) pan:         CustomInput,
    pub(super) trackpad:    CustomInput,
    pub(super) zoom_coarse: CustomInput,
    pub(super) zoom_smooth: CustomInput,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TrackpadBindingCondition {
    pub(super) target:   TrackpadScrollTarget,
    pub(super) index:    usize,
    pub(super) mod_keys: ModKeys,
    pub(super) active:   bool,
}

impl TrackpadBindingCondition {
    const fn new(
        target: TrackpadScrollTarget,
        index: usize,
        binding: OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>,
    ) -> Self {
        Self {
            target,
            index,
            mod_keys: binding.binding().mod_keys,
            active: false,
        }
    }
}

impl InputCondition for TrackpadBindingCondition {
    fn evaluate(&mut self, _: &ActionsQuery, _: &ContextTime, value: ActionValue) -> TriggerState {
        if self.active && value.as_bool() {
            TriggerState::Fired
        } else {
            TriggerState::None
        }
    }
}

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct OrbitCamAdapterDiagnostics {
    pub(super) installed_cameras:  usize,
    pub(super) installed_entities: usize,
    pub(super) gated_cameras:      usize,
}

pub(super) fn clear_replaced_or_manual_installations(world: &mut World) {
    let mut query = world.query_filtered::<Entity, With<OrbitCamInputContext>>();
    let cameras = query.iter(world).collect::<Vec<_>>();

    for camera in cameras {
        let installed_entities = input::installed_input_entities_for::<OrbitCamKind>(world, camera);
        if installed_entities.is_empty()
            || input::input_installation_has_placeholder_for::<OrbitCamKind>(world, camera)
        {
            clear_enhanced_input_components(world, camera);
        }
    }
}

fn clear_enhanced_input_components(world: &mut World, camera: Entity) {
    let mut entity = world.entity_mut(camera);
    entity
        .remove::<OrbitCamInputContext>()
        .remove::<ContextActivity<OrbitCamInputContext>>()
        .remove::<ContextPriority<OrbitCamInputContext>>()
        .remove::<GamepadDevice>()
        .remove::<Actions<OrbitCamInputContext>>()
        .remove::<OrbitCamInputActionEntities>()
        .remove::<CameraInstalledBindings<OrbitCamKind>>()
        .remove::<OrbitCamAdapterCustomInputs>()
        .remove::<OrbitCamAdapterFrameSources>();
}

pub(super) fn install_enhanced_input_entities(world: &mut World) {
    let mut query = world.query_filtered::<(Entity, &OrbitCamResolvedBindings), With<OrbitCam>>();
    let cameras = query
        .iter(world)
        .filter(|(camera, _)| {
            input::input_installation_has_placeholder_for::<OrbitCamKind>(world, *camera)
        })
        .map(|(camera, bindings)| (camera, bindings.0.clone()))
        .collect::<Vec<_>>();

    let mut installed_cameras = 0;
    let mut installed_entities = 0;

    for (camera, bindings) in cameras {
        for installed_entity in input::installed_input_entities_for::<OrbitCamKind>(world, camera) {
            let _ = world.despawn(installed_entity);
        }

        let installation = spawn_input_installation(world, camera, &bindings);
        installed_entities += installation.entities.len();
        installed_cameras += 1;

        world.entity_mut(camera).insert((
            OrbitCamInputContext,
            gamepad_device_for(&bindings),
            CameraInstalledBindings::<OrbitCamKind>(bindings),
            installation.actions,
            installation.custom_inputs,
            OrbitCamAdapterFrameSources::default(),
        ));
        input::replace_installed_input_entities_for::<OrbitCamKind>(
            world,
            camera,
            installation.entities,
        );
    }

    let mut diagnostics = world.resource_mut::<OrbitCamAdapterDiagnostics>();
    diagnostics.installed_cameras = installed_cameras;
    diagnostics.installed_entities = installed_entities;
}

const fn gamepad_device_for(bindings: &OrbitCamBindings) -> GamepadDevice {
    match bindings.gamepad() {
        CameraInputGamepadSelectionPolicy::Disabled => GamepadDevice::None,
        CameraInputGamepadSelectionPolicy::Active => GamepadDevice::Any,
    }
}

struct SpawnedInputInstallation {
    actions:       OrbitCamInputActionEntities,
    custom_inputs: OrbitCamAdapterCustomInputs,
    entities:      Vec<Entity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SpawnedInputActions {
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
}

impl SpawnedInputActions {
    fn entities(self) -> Vec<Entity> {
        vec![
            self.orbit,
            self.orbit_slow,
            self.orbit_engaged,
            self.pan,
            self.pan_slow,
            self.pan_engaged,
            self.zoom_coarse,
            self.zoom_smooth,
            self.zoom_smooth_slow,
            self.zoom_engaged,
            self.slow_mode_toggle,
            self.adapter_orbit,
            self.adapter_pan,
            self.adapter_zoom_coarse,
            self.adapter_zoom_smooth,
            self.home,
        ]
    }

    const fn adapter_actions(self) -> (Entity, Entity, Entity, Entity) {
        (
            self.adapter_orbit,
            self.adapter_pan,
            self.adapter_zoom_coarse,
            self.adapter_zoom_smooth,
        )
    }

    const fn trackpad_actions(self) -> (Entity, Entity, Entity) {
        (
            self.adapter_orbit,
            self.adapter_pan,
            self.adapter_zoom_smooth,
        )
    }

    fn with_sources(self, bindings: &OrbitCamBindings) -> OrbitCamInputActionEntities {
        OrbitCamInputActionEntities {
            orbit:               self.orbit,
            orbit_slow:          self.orbit_slow,
            orbit_engaged:       self.orbit_engaged,
            pan:                 self.pan,
            pan_slow:            self.pan_slow,
            pan_engaged:         self.pan_engaged,
            zoom_coarse:         self.zoom_coarse,
            zoom_smooth:         self.zoom_smooth,
            zoom_smooth_slow:    self.zoom_smooth_slow,
            zoom_engaged:        self.zoom_engaged,
            slow_mode_toggle:    self.slow_mode_toggle,
            adapter_orbit:       self.adapter_orbit,
            adapter_pan:         self.adapter_pan,
            adapter_zoom_coarse: self.adapter_zoom_coarse,
            adapter_zoom_smooth: self.adapter_zoom_smooth,
            home:                self.home,
            orbit_sources:       input::held_sources(bindings.orbit().enabled_entries()),
            pan_sources:         input::held_sources(bindings.pan().enabled_entries()),
            zoom_coarse_sources: input::action_sources(bindings.zoom_coarse().enabled_entries()),
            zoom_smooth_sources: input::held_sources(bindings.zoom_smooth().enabled_entries()),
            home_sources:        input::action_sources(bindings.enabled_home_entries()),
            home_state:          HomeActionState::Inactive,
        }
    }
}

fn spawn_input_installation(
    world: &mut World,
    camera: Entity,
    bindings: &OrbitCamBindings,
) -> SpawnedInputInstallation {
    let custom_inputs = register_adapter_custom_inputs(world);
    let actions = spawn_input_actions(world, camera);
    let mut entities = actions.entities();
    spawn_adapter_custom_bindings(
        world,
        camera,
        &custom_inputs,
        actions.adapter_actions(),
        &mut entities,
    );
    spawn_trackpad_custom_bindings(
        world,
        camera,
        &custom_inputs,
        actions.trackpad_actions(),
        bindings,
        &mut entities,
    );
    spawn_camera_action_bindings(world, camera, bindings, actions, &mut entities);

    SpawnedInputInstallation {
        actions: actions.with_sources(bindings),
        custom_inputs,
        entities,
    }
}

fn spawn_input_actions(world: &mut World, camera: Entity) -> SpawnedInputActions {
    SpawnedInputActions {
        orbit:               input::spawn_action::<OrbitCamOrbitAction, OrbitCamKind>(
            world, camera,
        ),
        orbit_slow:          input::spawn_action::<OrbitCamOrbitSlowAction, OrbitCamKind>(
            world, camera,
        ),
        orbit_engaged:       input::spawn_action::<OrbitCamOrbitEngagedAction, OrbitCamKind>(
            world, camera,
        ),
        pan:                 input::spawn_action::<OrbitCamPanAction, OrbitCamKind>(world, camera),
        pan_slow:            input::spawn_action::<OrbitCamPanSlowAction, OrbitCamKind>(
            world, camera,
        ),
        pan_engaged:         input::spawn_action::<OrbitCamPanEngagedAction, OrbitCamKind>(
            world, camera,
        ),
        zoom_coarse:         input::spawn_action::<OrbitCamZoomCoarseAction, OrbitCamKind>(
            world, camera,
        ),
        zoom_smooth:         input::spawn_action::<OrbitCamZoomSmoothAction, OrbitCamKind>(
            world, camera,
        ),
        zoom_smooth_slow:    input::spawn_action::<OrbitCamZoomSmoothSlowAction, OrbitCamKind>(
            world, camera,
        ),
        zoom_engaged:        input::spawn_action::<OrbitCamZoomEngagedAction, OrbitCamKind>(
            world, camera,
        ),
        slow_mode_toggle:    input::spawn_action::<OrbitCamSlowModeToggleAction, OrbitCamKind>(
            world, camera,
        ),
        adapter_orbit:       input::spawn_action::<OrbitCamAdapterOrbitAction, OrbitCamKind>(
            world, camera,
        ),
        adapter_pan:         input::spawn_action::<OrbitCamAdapterPanAction, OrbitCamKind>(
            world, camera,
        ),
        adapter_zoom_coarse: input::spawn_action::<OrbitCamAdapterZoomCoarseAction, OrbitCamKind>(
            world, camera,
        ),
        adapter_zoom_smooth: input::spawn_action::<OrbitCamAdapterZoomSmoothAction, OrbitCamKind>(
            world, camera,
        ),
        home:                input::spawn_action::<OrbitCamHomeAction, OrbitCamKind>(world, camera),
    }
}

fn spawn_camera_action_bindings(
    world: &mut World,
    camera: Entity,
    bindings: &OrbitCamBindings,
    actions: SpawnedInputActions,
    entities: &mut Vec<Entity>,
) {
    let mut gate_actions = GateActionCache::default();
    input::spawn_held_bindings::<_, OrbitCamKind>(
        world,
        camera,
        MotionActions {
            normal: actions.orbit,
            slow:   actions.orbit_slow,
        },
        actions.orbit_engaged,
        bindings.orbit().enabled_entries(),
        &mut gate_actions,
        entities,
    );
    input::spawn_held_bindings::<_, OrbitCamKind>(
        world,
        camera,
        MotionActions {
            normal: actions.pan,
            slow:   actions.pan_slow,
        },
        actions.pan_engaged,
        bindings.pan().enabled_entries(),
        &mut gate_actions,
        entities,
    );
    input::spawn_held_bindings::<_, OrbitCamKind>(
        world,
        camera,
        MotionActions {
            normal: actions.zoom_smooth,
            slow:   actions.zoom_smooth_slow,
        },
        actions.zoom_engaged,
        bindings.zoom_smooth().enabled_entries(),
        &mut gate_actions,
        entities,
    );
    for entry in bindings.zoom_coarse().enabled_entries() {
        input::spawn_binding::<OrbitCamKind>(
            world,
            camera,
            actions.zoom_coarse,
            entry.binding_descriptor(),
            &BindingGates::default(),
            &mut gate_actions,
            entities,
        );
    }
    if let Some(slow_mode) = bindings.slow_mode() {
        world
            .entity_mut(actions.slow_mode_toggle)
            .insert(Press::default());
        let entity = input::spawn_single_binding::<OrbitCamKind>(
            world,
            actions.slow_mode_toggle,
            camera,
            Binding::Keyboard {
                key:      slow_mode.toggle_key,
                mod_keys: slow_mode.mod_keys,
            },
        );
        entities.push(entity);
    }
    for entry in bindings.enabled_home_entries() {
        input::spawn_binding::<OrbitCamKind>(
            world,
            camera,
            actions.home,
            entry.binding_descriptor(),
            &BindingGates::default(),
            &mut gate_actions,
            entities,
        );
    }
}

fn register_adapter_custom_inputs(world: &mut World) -> OrbitCamAdapterCustomInputs {
    let mut custom_inputs = world.resource_mut::<CustomInputs>();
    OrbitCamAdapterCustomInputs {
        orbit:       custom_inputs.register_input(),
        pan:         custom_inputs.register_input(),
        trackpad:    custom_inputs.register_input(),
        zoom_coarse: custom_inputs.register_input(),
        zoom_smooth: custom_inputs.register_input(),
    }
}

fn spawn_adapter_custom_bindings(
    world: &mut World,
    camera: Entity,
    custom_inputs: &OrbitCamAdapterCustomInputs,
    actions: (Entity, Entity, Entity, Entity),
    entities: &mut Vec<Entity>,
) {
    let (orbit, pan, zoom_coarse, zoom_smooth) = actions;
    entities.push(input::spawn_single_binding::<OrbitCamKind>(
        world,
        orbit,
        camera,
        Binding::Custom(custom_inputs.orbit),
    ));
    entities.push(input::spawn_single_binding::<OrbitCamKind>(
        world,
        pan,
        camera,
        Binding::Custom(custom_inputs.pan),
    ));
    entities.push(input::spawn_single_binding::<OrbitCamKind>(
        world,
        zoom_coarse,
        camera,
        Binding::Custom(custom_inputs.zoom_coarse),
    ));
    entities.push(input::spawn_single_binding::<OrbitCamKind>(
        world,
        zoom_smooth,
        camera,
        Binding::Custom(custom_inputs.zoom_smooth),
    ));
}

fn spawn_trackpad_custom_bindings(
    world: &mut World,
    camera: Entity,
    custom_inputs: &OrbitCamAdapterCustomInputs,
    actions: (Entity, Entity, Entity),
    bindings: &OrbitCamBindings,
    entities: &mut Vec<Entity>,
) {
    let (orbit, pan, zoom_smooth) = actions;
    for (index, binding) in bindings.enabled_trackpad_orbit() {
        entities.push(spawn_trackpad_binding(
            world,
            camera,
            orbit,
            custom_inputs.trackpad,
            TrackpadScrollTarget::Orbit,
            index,
            binding,
        ));
    }
    for (index, binding) in bindings.enabled_trackpad_pan() {
        entities.push(spawn_trackpad_binding(
            world,
            camera,
            pan,
            custom_inputs.trackpad,
            TrackpadScrollTarget::Pan,
            index,
            binding,
        ));
    }
    for (index, binding) in bindings.enabled_trackpad_zoom() {
        entities.push(spawn_trackpad_zoom_binding(
            world,
            camera,
            zoom_smooth,
            custom_inputs.trackpad,
            index,
            binding,
            bindings.zoom_inversion(),
        ));
    }
}

/// Spawns a trackpad scroll binding whose two scroll axes drive orbit or pan
/// directly, with no zoom conversion.
fn spawn_trackpad_binding(
    world: &mut World,
    camera: Entity,
    action: Entity,
    input: CustomInput,
    target: TrackpadScrollTarget,
    index: usize,
    binding: OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>,
) -> Entity {
    let entity =
        input::spawn_single_binding::<OrbitCamKind>(world, action, camera, Binding::Custom(input));
    world
        .entity_mut(entity)
        .insert(Scale::splat(binding.input_gain().value()));
    insert_trackpad_condition(world, entity, target, index, binding)
}

/// Spawns a trackpad scroll binding that drives zoom: the vertical scroll axis
/// is swizzled onto the zoom axis and scaled, then negated when the camera's
/// zoom is inverted.
fn spawn_trackpad_zoom_binding(
    world: &mut World,
    camera: Entity,
    action: Entity,
    input: CustomInput,
    index: usize,
    binding: OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>,
    zoom_inversion: ZoomInversion,
) -> Entity {
    let entity =
        input::spawn_single_binding::<OrbitCamKind>(world, action, camera, Binding::Custom(input));
    let scale = PIXEL_SCROLL_SCALE * binding.input_gain().value();
    world
        .entity_mut(entity)
        .insert((SwizzleAxis::YXZ, Scale::splat(scale)));
    if matches!(zoom_inversion, ZoomInversion::Inverted) {
        world.entity_mut(entity).insert(Negate::all());
    }
    insert_trackpad_condition(world, entity, TrackpadScrollTarget::Zoom, index, binding)
}

fn insert_trackpad_condition(
    world: &mut World,
    entity: Entity,
    target: TrackpadScrollTarget,
    index: usize,
    binding: OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>,
) -> Entity {
    world
        .entity_mut(entity)
        .insert(TrackpadBindingCondition::new(target, index, binding));
    entity
}

pub(super) fn apply_context_gating(world: &mut World) {
    let mut query = world.query_filtered::<(
        Entity,
        &CameraInputContextGated,
        Option<&ContextActivity<OrbitCamInputContext>>,
    ), With<OrbitCamInputContext>>();
    let updates = query
        .iter(world)
        .filter_map(|(camera, gated, current)| {
            let allowed = gated.context_gate.is_allowed();
            let desired = ContextActivity::<OrbitCamInputContext>::new(allowed);
            if current.is_none_or(|current| **current != allowed) {
                Some((camera, desired))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let gated_cameras = updates.len();

    for (camera, desired) in updates {
        world.entity_mut(camera).insert(desired);
    }

    world
        .resource_mut::<OrbitCamAdapterDiagnostics>()
        .gated_cameras = gated_cameras;
}
