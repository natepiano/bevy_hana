//! Camera-agnostic driver that spawns `bevy_enhanced_input` action and binding
//! entities from a validated binding set.
//!
//! Each camera kind's installation system builds the action entities it needs
//! and then calls these functions to attach bindings, modifiers, and gate
//! conditions. A camera kind supplies its enhanced-input context, its
//! installation-marker component, and its gate action through
//! [`CameraInstallKind`]; everything else here is identical across cameras.
//!
//! Types:
//! - [`CameraInstallKind`] — the per-camera install hook.
//! - [`CameraBindingGateCondition`] — enhanced-input condition that fires a binding only when its
//!   gate actions are in the required state.
//! - [`CameraWheelBindingCondition`] — enhanced-input blocker that withholds a mouse-wheel binding
//!   while its camera's blockers withhold scroll.
//! - [`MotionActions`] — the normal/slow action pair a held motion binding routes to by
//!   [`ControlSpeed`].
//! - [`GateActionCache`] — dedups the gate action spawned per distinct [`GateInput`] within one
//!   installation.

use std::collections::HashMap;

use bevy::prelude::ButtonInput;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::World;
use bevy_enhanced_input::prelude::Accumulation;
use bevy_enhanced_input::prelude::Action;
use bevy_enhanced_input::prelude::ActionOf;
use bevy_enhanced_input::prelude::ActionSettings;
use bevy_enhanced_input::prelude::ActionSources;
use bevy_enhanced_input::prelude::ActionValue;
use bevy_enhanced_input::prelude::ActionsQuery;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::BindingOf;
use bevy_enhanced_input::prelude::ConditionKind;
use bevy_enhanced_input::prelude::ContextTime;
use bevy_enhanced_input::prelude::DeadZone;
use bevy_enhanced_input::prelude::DeadZoneKind;
use bevy_enhanced_input::prelude::DeltaScale;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::InputCondition;
use bevy_enhanced_input::prelude::ModKeys;
use bevy_enhanced_input::prelude::Negate;
use bevy_enhanced_input::prelude::Scale;
use bevy_enhanced_input::prelude::SwizzleAxis;
use bevy_enhanced_input::prelude::TriggerState;

use super::CameraInputBlockers;
use super::CameraInputModeKind;
use super::CameraSemanticAction;
use super::ControlSpeed;
use super::HeldCameraAction;
use super::InteractionSources;
use super::ResolvedCameraInputRoute;
use super::bindings::BindingGates;
use super::bindings::GateInput;
use super::bindings::GatePolarity;
use super::bindings::HeldActionBindingEntry;
use super::bindings::ImpulseActionBindingEntry;
use super::bindings::InputAxisTransform;
use super::bindings::InputBindingDescriptor;
use super::bindings::InputBindingEntry;
use super::bindings::InputBindingModifiers;
use super::bindings::InputDeadZone;
use super::bindings::InputDeltaScale;
use super::mode_reconciliation::CameraInputInstallationOf;
use super::pointer_motion::CursorDragInput;
use super::pointer_motion::DragMotionSource;

/// Per-camera hook that lets the shared install driver spawn action entities
/// under the right enhanced-input context, tag them with the camera's
/// installation marker, and spawn the camera's gate action.
pub(crate) trait CameraInstallKind: CameraInputModeKind {
    /// The bool action spawned for a gate key or button.
    /// [`CameraBindingGateCondition`] treats the gate as actuated while this
    /// action's [`TriggerState`] is `Ongoing` or `Fired`.
    type GateAction: InputAction<Output = bool>;
}

/// Enhanced-input condition that fires a gated binding only when every gate
/// action is in the state its polarity requires.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub(crate) struct CameraBindingGateCondition {
    gates: Vec<InstalledBindingGate>,
}

impl CameraBindingGateCondition {
    fn new(gates: impl Into<Vec<InstalledBindingGate>>) -> Self {
        Self {
            gates: gates.into(),
        }
    }
}

impl InputCondition for CameraBindingGateCondition {
    fn evaluate(
        &mut self,
        actions: &ActionsQuery,
        _: &ContextTime,
        value: ActionValue,
    ) -> TriggerState {
        let actuated = value.as_bool();
        if !actuated {
            return TriggerState::None;
        }

        let gates_satisfied = self.gates.iter().all(|gate| {
            let active = actions.get(gate.action).is_ok_and(|(_, state, ..)| {
                matches!(*state, TriggerState::Ongoing | TriggerState::Fired)
            });
            match gate.polarity {
                GatePolarity::Required => active,
                GatePolarity::Blocked => !active,
            }
        });
        if gates_satisfied {
            TriggerState::Fired
        } else {
            TriggerState::None
        }
    }

    fn kind(&self) -> ConditionKind { ConditionKind::Implicit }
}

/// Enhanced-input blocker on every mouse-wheel binding: withholds the binding
/// while its camera's blockers withhold scroll, so a wheel bound through
/// enhanced input obeys [`CameraScrollInputDisabled`](super::CameraScrollInputDisabled)
/// exactly as the adapter's wheel bindings do. Bindings on other devices keep
/// resolving.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CameraWheelBindingCondition {
    camera:   Entity,
    delivery: WheelDelivery,
}

/// Whether a camera's wheel bindings receive the wheel this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WheelDelivery {
    Delivered,
    Withheld,
}

impl InputCondition for CameraWheelBindingCondition {
    fn evaluate(&mut self, _: &ActionsQuery, _: &ContextTime, _: ActionValue) -> TriggerState {
        match self.delivery {
            WheelDelivery::Delivered => TriggerState::Fired,
            WheelDelivery::Withheld => TriggerState::None,
        }
    }

    fn kind(&self) -> ConditionKind { ConditionKind::Blocker }
}

/// Withholds each camera's wheel bindings while its blockers withhold scroll.
///
/// Runs after routing resolves the blockers and before enhanced input
/// evaluates the bindings.
pub(crate) fn withhold_wheel_bindings_of_scroll_blocked_cameras(
    route: Res<ResolvedCameraInputRoute>,
    mut conditions: Query<&mut CameraWheelBindingCondition>,
) {
    for mut condition in &mut conditions {
        let delivery = if route
            .blockers_for(condition.camera)
            .is_some_and(CameraInputBlockers::blocks_scroll)
        {
            WheelDelivery::Withheld
        } else {
            WheelDelivery::Delivered
        };
        if condition.delivery != delivery {
            condition.delivery = delivery;
        }
    }
}

/// Withholds mouse-button and mouse-motion bindings during a claimed drag.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CameraDragBindingCondition {
    camera:   Entity,
    delivery: DragDelivery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragDelivery {
    Delivered,
    Withheld,
}

impl InputCondition for CameraDragBindingCondition {
    fn evaluate(&mut self, _: &ActionsQuery, _: &ContextTime, _: ActionValue) -> TriggerState {
        match self.delivery {
            DragDelivery::Delivered => TriggerState::Fired,
            DragDelivery::Withheld => TriggerState::None,
        }
    }

    fn kind(&self) -> ConditionKind { ConditionKind::Blocker }
}

pub(crate) fn withhold_drag_bindings_of_claimed_cameras(
    route: Res<ResolvedCameraInputRoute>,
    mut conditions: Query<&mut CameraDragBindingCondition>,
) {
    for mut condition in &mut conditions {
        condition.delivery = if route
            .blockers_for(condition.camera)
            .is_some_and(CameraInputBlockers::blocks_pointer_drag)
        {
            DragDelivery::Withheld
        } else {
            DragDelivery::Delivered
        };
    }
}

/// Allows only the sibling binding selected for the current camera drag.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CameraMotionSourceCondition {
    camera:    Entity,
    source:    DragMotionSource,
    delivered: bool,
}

impl CameraMotionSourceCondition {
    pub(super) const fn camera(&self) -> Entity { self.camera }

    pub(super) fn select(&mut self, camera_is_routed: bool, source: DragMotionSource) {
        self.delivered = if camera_is_routed {
            match self.source {
                DragMotionSource::RawMotion => source != DragMotionSource::CursorDelta,
                DragMotionSource::CursorDelta => source == DragMotionSource::CursorDelta,
                DragMotionSource::Undecided => false,
            }
        } else {
            self.source == DragMotionSource::RawMotion
        };
    }
}

impl InputCondition for CameraMotionSourceCondition {
    fn evaluate(&mut self, _: &ActionsQuery, _: &ContextTime, _: ActionValue) -> TriggerState {
        if self.delivered {
            TriggerState::Fired
        } else {
            TriggerState::None
        }
    }

    fn kind(&self) -> ConditionKind { ConditionKind::Blocker }
}

/// Applies the same required-modifier test as native mouse motion.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CameraCursorModifierCondition {
    mod_keys: ModKeys,
    pressed:  bool,
}

impl InputCondition for CameraCursorModifierCondition {
    fn evaluate(&mut self, _: &ActionsQuery, _: &ContextTime, _: ActionValue) -> TriggerState {
        if self.pressed {
            TriggerState::Fired
        } else {
            TriggerState::None
        }
    }

    fn kind(&self) -> ConditionKind { ConditionKind::Blocker }
}

pub(super) fn refresh_cursor_modifiers(
    keyboard: Option<Res<ButtonInput<KeyCode>>>,
    action_sources: Res<ActionSources>,
    mut conditions: Query<&mut CameraCursorModifierCondition>,
) {
    for mut condition in &mut conditions {
        condition.pressed = (condition.mod_keys.is_empty() || action_sources.keyboard)
            && condition.mod_keys.iter_keys().all(|pair| {
                keyboard
                    .as_deref()
                    .is_some_and(|keys| keys.any_pressed(pair))
            });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InstalledBindingGate {
    action:   Entity,
    polarity: GatePolarity,
}

/// The normal/slow motion action pair a held motion binding routes to by speed.
#[derive(Clone, Copy)]
pub(crate) struct MotionActions {
    pub(crate) normal: Entity,
    pub(crate) slow:   Entity,
}

impl MotionActions {
    const fn for_speed(self, speed: ControlSpeed) -> Entity {
        match speed {
            ControlSpeed::Normal => self.normal,
            ControlSpeed::Slow => self.slow,
        }
    }
}

/// Dedups the gate action spawned per distinct [`GateInput`] within one
/// installation, so a key or button that gates several bindings drives a single
/// action entity.
#[derive(Default)]
pub(crate) struct GateActionCache {
    actions: HashMap<GateInput, Entity>,
}

impl GateActionCache {
    /// Returns the gate action installed for `input`, if any binding gated on it.
    pub(crate) fn action_for(&self, input: GateInput) -> Option<Entity> {
        self.actions.get(&input).copied()
    }
}

/// Spawns an action entity under the camera's enhanced-input context, tagged
/// with the camera's installation marker.
pub(crate) fn spawn_action<A, K>(world: &mut World, camera: Entity) -> Entity
where
    A: InputAction,
    K: CameraInstallKind,
{
    world
        .spawn((
            Action::<A>::new(),
            ActionOf::<K::Context>::new(camera),
            action_settings(),
            CameraInputInstallationOf::<K>::new(camera),
        ))
        .id()
}

const fn action_settings() -> ActionSettings {
    ActionSettings {
        accumulation:  Accumulation::Cumulative,
        require_reset: false,
        consume_input: false,
    }
}

/// Spawns every enabled entry of a held action binding set: the motion binding
/// routes to the action for the entry's speed, so the reported speed comes from
/// which motion action fired, and the engagement binding routes to the
/// engagement action. Both carry the entry's gate conditions.
pub(crate) fn spawn_held_bindings<'a, A, K>(
    world: &mut World,
    camera: Entity,
    motion_actions: MotionActions,
    engagement_action: Entity,
    entries: impl IntoIterator<Item = &'a HeldActionBindingEntry<A>>,
    gate_actions: &mut GateActionCache,
    entities: &mut Vec<Entity>,
) where
    A: HeldCameraAction + 'a,
    K: CameraInstallKind,
{
    for entry in entries {
        spawn_binding::<K>(
            world,
            camera,
            motion_actions.for_speed(entry.speed()),
            entry.motion_descriptor(),
            entry.gates(),
            gate_actions,
            entities,
        );
        spawn_binding::<K>(
            world,
            camera,
            engagement_action,
            entry.engagement_descriptor(),
            entry.gates(),
            gate_actions,
            entities,
        );
    }
}

/// Spawns every enabled entry of a binding descriptor onto one action, attaching
/// the descriptor's gate conditions to each spawned binding.
pub(crate) fn spawn_binding<K: CameraInstallKind>(
    world: &mut World,
    camera: Entity,
    action: Entity,
    binding_descriptor: &InputBindingDescriptor,
    gates: &BindingGates,
    gate_actions: &mut GateActionCache,
    entities: &mut Vec<Entity>,
) {
    let gate_entities = gate_condition_entities::<K>(world, camera, gates, gate_actions, entities);
    for entry in binding_descriptor.enabled_entries() {
        for binding in spawn_binding_entry::<K>(world, action, camera, entry) {
            if !gate_entities.is_empty() {
                world
                    .entity_mut(binding)
                    .insert(CameraBindingGateCondition::new(gate_entities.clone()));
            }
            entities.push(binding);
        }
    }
}

fn spawn_binding_entry<K: CameraInstallKind>(
    world: &mut World,
    action: Entity,
    camera: Entity,
    entry: &InputBindingEntry,
) -> Vec<Entity> {
    let binding = entry.binding();
    let mut entities = vec![spawn_single_binding::<K>(world, action, camera, binding)];
    if let Binding::MouseMotion { mod_keys } = binding {
        let cursor_input = world.resource::<CursorDragInput>().0;
        let cursor =
            spawn_single_binding::<K>(world, action, camera, Binding::Custom(cursor_input));
        world.entity_mut(cursor).insert((
            CameraDragBindingCondition {
                camera,
                delivery: DragDelivery::Delivered,
            },
            CameraCursorModifierCondition {
                mod_keys,
                pressed: mod_keys.is_empty(),
            },
        ));
        world
            .entity_mut(entities[0])
            .insert(CameraMotionSourceCondition {
                camera,
                source: DragMotionSource::RawMotion,
                delivered: true,
            });
        world
            .entity_mut(cursor)
            .insert(CameraMotionSourceCondition {
                camera,
                source: DragMotionSource::CursorDelta,
                delivered: false,
            });
        entities.push(cursor);
    }
    let modifiers = entry.install_modifiers();
    for &entity in &entities {
        install_entry_modifiers(world, entity, modifiers);
    }
    entities
}

fn install_entry_modifiers(world: &mut World, entity: Entity, modifiers: InputBindingModifiers) {
    if let Some(dead_zone) = modifiers.dead_zone() {
        world
            .entity_mut(entity)
            .insert(dead_zone_modifier(dead_zone));
    }
    if let Some(scale) = modifiers.scale() {
        world.entity_mut(entity).insert(Scale::splat(scale));
    }
    if modifiers.delta_scale() == InputDeltaScale::Auto {
        world.entity_mut(entity).insert(DeltaScale::AUTO);
    }
    match modifiers.axis_transform() {
        InputAxisTransform::None => {},
        InputAxisTransform::Negate => {
            world.entity_mut(entity).insert(Negate::all());
        },
        InputAxisTransform::Swizzle => {
            world.entity_mut(entity).insert(SwizzleAxis::YXZ);
        },
        InputAxisTransform::SwizzleNegate => {
            world
                .entity_mut(entity)
                .insert((SwizzleAxis::YXZ, Negate::all()));
        },
        InputAxisTransform::SwizzleZ => {
            world.entity_mut(entity).insert(SwizzleAxis::ZYX);
        },
        InputAxisTransform::SwizzleZNegate => {
            world
                .entity_mut(entity)
                .insert((SwizzleAxis::ZYX, Negate::all()));
        },
    }
}

/// Spawns a single binding entity for one enhanced-input [`Binding`], tagged
/// with the camera's installation marker.
pub(crate) fn spawn_single_binding<K: CameraInstallKind>(
    world: &mut World,
    action: Entity,
    camera: Entity,
    binding: Binding,
) -> Entity {
    let mut entity = world.spawn((
        binding,
        BindingOf(action),
        CameraInputInstallationOf::<K>::new(camera),
    ));
    if matches!(binding, Binding::MouseWheel { .. }) {
        entity.insert(CameraWheelBindingCondition {
            camera,
            delivery: WheelDelivery::Delivered,
        });
    }
    if matches!(
        binding,
        Binding::MouseButton { .. } | Binding::MouseMotion { .. }
    ) {
        entity.insert(CameraDragBindingCondition {
            camera,
            delivery: DragDelivery::Delivered,
        });
    }
    entity.id()
}

const fn dead_zone_modifier(dead_zone: InputDeadZone) -> DeadZone {
    DeadZone {
        kind:            DeadZoneKind::Axial,
        lower_threshold: dead_zone.lower_threshold,
        upper_threshold: dead_zone.upper_threshold,
    }
}

fn gate_condition_entities<K: CameraInstallKind>(
    world: &mut World,
    camera: Entity,
    gates: &BindingGates,
    gate_actions: &mut GateActionCache,
    entities: &mut Vec<Entity>,
) -> Vec<InstalledBindingGate> {
    gates
        .entries()
        .iter()
        .map(|gate| InstalledBindingGate {
            action:   gate_action_entity::<K>(world, camera, gate.input, gate_actions, entities),
            polarity: gate.polarity,
        })
        .collect()
}

fn gate_action_entity<K: CameraInstallKind>(
    world: &mut World,
    camera: Entity,
    input: GateInput,
    gate_actions: &mut GateActionCache,
    entities: &mut Vec<Entity>,
) -> Entity {
    if let Some(action) = gate_actions.actions.get(&input) {
        return *action;
    }

    let action = spawn_action::<K::GateAction, K>(world, camera);
    let binding = spawn_single_binding::<K>(world, action, camera, binding_for_gate_input(input));
    entities.push(action);
    entities.push(binding);
    gate_actions.actions.insert(input, action);
    action
}

const fn binding_for_gate_input(input: GateInput) -> Binding {
    match input {
        GateInput::GamepadButton(button) => Binding::GamepadButton(button),
        GateInput::Key(key) => Binding::Keyboard {
            key,
            mod_keys: ModKeys::empty(),
        },
    }
}

/// Unions the active sources of every enabled held binding entry.
pub(crate) fn held_sources<'a, A: HeldCameraAction + 'a>(
    entries: impl IntoIterator<Item = &'a HeldActionBindingEntry<A>>,
) -> InteractionSources {
    entries
        .into_iter()
        .fold(InteractionSources::NONE, |sources, entry| {
            sources.union(entry.sources())
        })
}

/// Unions the active sources of every enabled action binding entry.
pub(crate) fn action_sources<'a, A>(
    entries: impl IntoIterator<Item = &'a ImpulseActionBindingEntry<A>>,
) -> InteractionSources
where
    A: CameraSemanticAction + 'a,
{
    entries
        .into_iter()
        .fold(InteractionSources::NONE, |sources, entry| {
            sources.union(entry.sources())
        })
}

#[cfg(test)]
mod tests {
    use bevy::prelude::*;
    use bevy_enhanced_input::prelude::ActionSources;
    use bevy_enhanced_input::prelude::ModKeys;

    use super::CameraCursorModifierCondition;
    use super::refresh_cursor_modifiers;

    #[test]
    fn keyboard_source_controls_required_cursor_modifiers() {
        let mut app = App::new();
        app.init_resource::<ActionSources>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_systems(Update, refresh_cursor_modifiers);
        let required = app
            .world_mut()
            .spawn(CameraCursorModifierCondition {
                mod_keys: ModKeys::SHIFT,
                pressed:  false,
            })
            .id();
        let unmodified = app
            .world_mut()
            .spawn(CameraCursorModifierCondition {
                mod_keys: ModKeys::empty(),
                pressed:  false,
            })
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        app.update();
        assert!(
            app.world()
                .get::<CameraCursorModifierCondition>(required)
                .is_some_and(|condition| condition.pressed)
        );

        app.world_mut().resource_mut::<ActionSources>().keyboard = false;
        app.update();
        assert!(
            !app.world()
                .get::<CameraCursorModifierCondition>(required)
                .is_some_and(|condition| condition.pressed)
        );
        assert!(
            app.world()
                .get::<CameraCursorModifierCondition>(unmodified)
                .is_some_and(|condition| condition.pressed)
        );
    }
}
