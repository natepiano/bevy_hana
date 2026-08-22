use bevy::ecs::system::SystemParam;
use bevy::input::gamepad::Gamepad;
use bevy::prelude::ButtonInput;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::MouseButton;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Without;
use bevy_enhanced_input::prelude::TriggerState;

use super::CameraInputContextGated;
use super::CameraInputModeKind;
use super::CameraInstalledBindings;
use super::CameraManual;
use super::CameraSlowModeLatches;
use super::CameraSlowModeState;
use super::InputIntent;
use super::LiveInputs;
use super::ResolvedCameraInputRoute;
use super::routing;

/// Camera-kind hook set for the shared enhanced-input action resolver.
pub(crate) trait CameraActionResolutionKind:
    CameraInputModeKind<Input = InputIntent<Self>> + Sized
{
    type ActionEntities: Component;
    type FrameState: Component;
    type ActionQueries<'w, 's>: SystemParam;

    fn slow_mode_toggle_action(_actions: &Self::ActionEntities) -> Option<Entity> { None }

    fn resolve_camera_actions(
        context: CameraActionResolutionContext<'_, Self>,
        action_queries: &Self::ActionQueries<'_, '_>,
        states: &Query<&TriggerState>,
        inputs: &LiveInputs<'_>,
    );
}

pub(crate) struct CameraActionResolutionContext<'a, K: CameraActionResolutionKind> {
    pub bindings:         &'a CameraInstalledBindings<K>,
    pub actions:          &'a K::ActionEntities,
    pub frame_state:      Option<&'a K::FrameState>,
    pub input:            &'a mut InputIntent<K>,
    pub slow_mode_active: bool,
}

#[derive(Component)]
pub(crate) struct NoActionFrameState;

pub(crate) fn resolve_actions_into_camera_input<K: CameraActionResolutionKind>(
    route: Res<ResolvedCameraInputRoute>,
    mut slow_latches: ResMut<CameraSlowModeLatches>,
    mut cameras: Query<
        (
            Entity,
            &CameraInstalledBindings<K>,
            &K::ActionEntities,
            Option<&K::FrameState>,
            Option<&CameraInputContextGated>,
            Option<&mut CameraSlowModeState>,
            &mut InputIntent<K>,
        ),
        Without<CameraManual<K>>,
    >,
    action_queries: K::ActionQueries<'_, '_>,
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

    for (camera, bindings, actions, frame_state, gated, slow_mode_state, mut input) in &mut cameras
    {
        input.clear();
        if route.routed_camera() != Some(camera)
            || route.metrics_for(camera).is_none()
            || route
                .blockers_for(camera)
                .is_some_and(super::CameraInputBlockers::is_blocked)
            || gated.is_some_and(|gated| !gated.context_gate.is_allowed())
        {
            continue;
        }

        if K::slow_mode_toggle_action(actions).is_some_and(|action| {
            states
                .get(action)
                .is_ok_and(|state| *state == TriggerState::Fired)
        }) {
            routing::toggle_slow_mode_latch(&mut slow_latches, camera);
        }
        let slow_mode_active = routing::is_slow_mode_active(&slow_latches, camera);
        if let Some(mut state) = slow_mode_state {
            state.set_active(slow_mode_active);
        }

        K::resolve_camera_actions(
            CameraActionResolutionContext {
                bindings,
                actions,
                frame_state,
                input: &mut input,
                slow_mode_active,
            },
            &action_queries,
            &states,
            &inputs,
        );
    }
}
