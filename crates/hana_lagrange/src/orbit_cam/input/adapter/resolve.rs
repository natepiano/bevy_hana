//! Reads the enhanced-input action state and writes the per-camera [`OrbitCamInput`].
//!
//! The generic action-resolution shell runs each frame in
//! `CameraInputInternalSet::ActionResolution`. For each routed, non-gated, non-manual
//! camera it samples the active engagement states, picks the source mask appropriate for
//! the engaged held bindings, and accumulates orbit / pan / zoom into the camera's
//! [`OrbitCamInput`] component — both from the user-bound actions and from the adapter-
//! synthesized actions written by [`super::inject`].
//!
//! Types (private — only used inside this module):
//! - [`Vec2ActionQueries`] / [`F32ActionQueries`] / [`BoolActionQueries`] — `SystemParam` bundles
//!   holding the per-action `Query<&Action<...>>` reads.
//! - [`HeldEngagement`] — `Engaged` / `Idle` enum derived from each engagement bool, used to
//!   short-circuit source selection.

use bevy::ecs::system::SystemParam;
use bevy::input::gamepad::Gamepad;
use bevy::prelude::ButtonInput;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::MouseButton;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::Vec2;
use bevy::prelude::With;
use bevy_enhanced_input::prelude::Action;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::ModKeys;
use bevy_enhanced_input::prelude::TriggerState;

use super::AdapterScale;
use super::HomeActionState;
use super::OrbitCamActionQueries;
use super::OrbitCamAdapterFrameSources;
use super::OrbitCamInputActionEntities;
use crate::OrbitCam;
use crate::OrbitCamHomePose;
use crate::OrbitCamKind;
use crate::camera_home::CameraHomeResetSources;
use crate::input;
use crate::input::CameraActionResolutionContext;
use crate::input::CameraActionResolutionKind;
use crate::input::CameraInstalledBindings;
use crate::input::ControlSpeed;
use crate::input::HeldActionBindingEntry;
use crate::input::HeldCameraAction;
use crate::input::InteractionSources;
use crate::input::LiveInputs;
use crate::input::OrbitCamAdapterOrbitAction;
use crate::input::OrbitCamAdapterPanAction;
use crate::input::OrbitCamAdapterZoomCoarseAction;
use crate::input::OrbitCamAdapterZoomSmoothAction;
use crate::input::OrbitCamBindings;
use crate::input::OrbitCamHomeAction;
use crate::input::OrbitCamOrbitAction;
use crate::input::OrbitCamOrbitEngagedAction;
use crate::input::OrbitCamOrbitSlowAction;
use crate::input::OrbitCamPanAction;
use crate::input::OrbitCamPanEngagedAction;
use crate::input::OrbitCamPanSlowAction;
use crate::input::OrbitCamZoomCoarseAction;
use crate::input::OrbitCamZoomEngagedAction;
use crate::input::OrbitCamZoomSmoothAction;
use crate::input::OrbitCamZoomSmoothSlowAction;
use crate::input::ResetOrbitCamToHome;
use crate::orbit_cam::OrbitCamInput;

impl CameraActionResolutionKind for OrbitCamKind {
    type ActionEntities = OrbitCamInputActionEntities;
    type FrameState = OrbitCamAdapterFrameSources;
    type ActionQueries<'w, 's> = OrbitCamActionQueries<'w, 's>;

    fn slow_mode_toggle_action(actions: &Self::ActionEntities) -> Option<Entity> {
        Some(actions.slow_mode_toggle)
    }

    fn resolve_camera_actions(
        context: CameraActionResolutionContext<'_, Self>,
        action_queries: &Self::ActionQueries<'_, '_>,
        states: &Query<&TriggerState>,
        inputs: &LiveInputs<'_>,
    ) {
        let CameraActionResolutionContext {
            bindings,
            actions,
            frame_state,
            input,
            slow_mode_active,
            ..
        } = context;
        let frame_sources = frame_state.copied().unwrap_or_default();
        let orbit_engaged =
            bool_action_active(actions.orbit_engaged, &action_queries.bools.orbit, states);
        let pan_engaged =
            bool_action_active(actions.pan_engaged, &action_queries.bools.pan, states);
        let zoom_engaged =
            bool_action_active(actions.zoom_engaged, &action_queries.bools.zoom, states);
        let pan_overrides_orbit = pan_overrides_orbit(&bindings.0, inputs);
        let adapter_scale = AdapterScale::from_bindings(&bindings.0, slow_mode_active);
        let orbit_sources = held_sources_for_state(
            HeldEngagement::from(orbit_engaged),
            bindings.0.orbit().enabled_entries(),
            actions.orbit_sources,
            inputs,
        );
        let pan_sources = held_sources_for_state(
            HeldEngagement::from(pan_engaged),
            bindings.0.pan().enabled_entries(),
            actions.pan_sources,
            inputs,
        );
        let zoom_smooth_sources = held_sources_for_state(
            HeldEngagement::from(zoom_engaged),
            bindings.0.zoom_smooth().enabled_entries(),
            actions.zoom_smooth_sources,
            inputs,
        );

        resolve_bound_motion_actions(
            actions,
            action_queries,
            states,
            input,
            adapter_scale,
            BoundMotionState {
                orbit: MotionEngagement {
                    active:  orbit_engaged,
                    sources: orbit_sources,
                },
                pan: MotionEngagement {
                    active:  pan_engaged,
                    sources: pan_sources,
                },
                zoom_smooth: MotionEngagement {
                    active:  zoom_engaged,
                    sources: zoom_smooth_sources,
                },
                pan_overrides_orbit,
            },
        );
        resolve_adapter_actions(
            actions,
            action_queries,
            states,
            input,
            adapter_scale,
            frame_sources,
        );
    }
}

/// Triggers an `OrbitCam` home reset on the home action's rising edge.
///
/// The reset observer performs the retarget and emits [`crate::input::CameraHomed`]
/// with the attributed sources stored by this system.
pub(super) fn apply_orbit_cam_home(
    mut commands: Commands,
    mut cameras: Query<
        (
            Entity,
            &mut OrbitCamInputActionEntities,
            &CameraInstalledBindings<OrbitCamKind>,
        ),
        (With<OrbitCam>, With<OrbitCamHomePose>),
    >,
    home_actions: Query<&Action<OrbitCamHomeAction>>,
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
                commands.trigger(ResetOrbitCamToHome { camera });
            }
            actions.home_state = next;
        }
    }
}

struct MotionEngagement {
    active:  bool,
    sources: InteractionSources,
}

struct BoundMotionState {
    orbit:               MotionEngagement,
    pan:                 MotionEngagement,
    zoom_smooth:         MotionEngagement,
    pan_overrides_orbit: bool,
}

fn resolve_bound_motion_actions(
    actions: &OrbitCamInputActionEntities,
    action_queries: &OrbitCamActionQueries,
    states: &Query<&TriggerState>,
    input: &mut OrbitCamInput,
    adapter_scale: AdapterScale,
    motion_state: BoundMotionState,
) {
    if motion_state.orbit.active && !motion_state.pan_overrides_orbit {
        let normal = action_value(actions.orbit, &action_queries.vec2.orbit);
        let slow = action_value(actions.orbit_slow, &action_queries.vec2.orbit_slow);
        input.add_orbit_with_sources(
            adapter_scale.vec2(normal + slow),
            motion_state.orbit.sources,
        );
        input.set_orbit_speed(vec2_speed(slow));
    }
    if motion_state.pan.active {
        let normal = action_value(actions.pan, &action_queries.vec2.pan);
        let slow = action_value(actions.pan_slow, &action_queries.vec2.pan_slow);
        input.add_pan_with_sources(adapter_scale.vec2(normal + slow), motion_state.pan.sources);
        input.set_pan_speed(vec2_speed(slow));
    }
    if motion_state.zoom_smooth.active {
        let normal = action_value(actions.zoom_smooth, &action_queries.f32.zoom_smooth);
        let slow = action_value(
            actions.zoom_smooth_slow,
            &action_queries.f32.zoom_smooth_slow,
        );
        input.add_zoom_smooth_with_sources(
            adapter_scale.f32(normal + slow),
            motion_state.zoom_smooth.sources,
        );
        input.set_zoom_speed(f32_speed(slow));
    }
    if action_state_active(actions.zoom_coarse, states) {
        input.add_zoom_coarse_with_sources(
            adapter_scale.f32(action_value(
                actions.zoom_coarse,
                &action_queries.f32.zoom_coarse,
            )),
            actions.zoom_coarse_sources,
        );
    }
}

fn resolve_adapter_actions(
    actions: &OrbitCamInputActionEntities,
    action_queries: &OrbitCamActionQueries,
    states: &Query<&TriggerState>,
    input: &mut OrbitCamInput,
    adapter_scale: AdapterScale,
    frame_sources: OrbitCamAdapterFrameSources,
) {
    if action_state_active(actions.adapter_orbit, states) {
        input.add_orbit_with_sources(
            adapter_scale.vec2(action_value(
                actions.adapter_orbit,
                &action_queries.vec2.adapter_orbit,
            )),
            frame_sources.orbit,
        );
    }
    if action_state_active(actions.adapter_pan, states) {
        input.add_pan_with_sources(
            adapter_scale.vec2(action_value(
                actions.adapter_pan,
                &action_queries.vec2.adapter_pan,
            )),
            frame_sources.pan,
        );
    }
    if action_state_active(actions.adapter_zoom_coarse, states) {
        input.add_zoom_coarse_with_sources(
            adapter_scale.f32(action_value(
                actions.adapter_zoom_coarse,
                &action_queries.f32.adapter_zoom_coarse,
            )),
            frame_sources.zoom_coarse,
        );
    }
    if action_state_active(actions.adapter_zoom_smooth, states) {
        input.add_zoom_smooth_with_sources(
            adapter_scale.f32(action_value(
                actions.adapter_zoom_smooth,
                &action_queries.f32.adapter_zoom_smooth,
            )),
            frame_sources.zoom_smooth,
        );
    }
}

#[derive(SystemParam)]
pub(super) struct Vec2ActionQueries<'w, 's> {
    orbit:         Query<'w, 's, &'static Action<OrbitCamOrbitAction>>,
    orbit_slow:    Query<'w, 's, &'static Action<OrbitCamOrbitSlowAction>>,
    pan:           Query<'w, 's, &'static Action<OrbitCamPanAction>>,
    pan_slow:      Query<'w, 's, &'static Action<OrbitCamPanSlowAction>>,
    adapter_orbit: Query<'w, 's, &'static Action<OrbitCamAdapterOrbitAction>>,
    adapter_pan:   Query<'w, 's, &'static Action<OrbitCamAdapterPanAction>>,
}

#[derive(SystemParam)]
pub(super) struct F32ActionQueries<'w, 's> {
    zoom_coarse:         Query<'w, 's, &'static Action<OrbitCamZoomCoarseAction>>,
    zoom_smooth:         Query<'w, 's, &'static Action<OrbitCamZoomSmoothAction>>,
    zoom_smooth_slow:    Query<'w, 's, &'static Action<OrbitCamZoomSmoothSlowAction>>,
    adapter_zoom_coarse: Query<'w, 's, &'static Action<OrbitCamAdapterZoomCoarseAction>>,
    adapter_zoom_smooth: Query<'w, 's, &'static Action<OrbitCamAdapterZoomSmoothAction>>,
}

#[derive(SystemParam)]
pub(super) struct BoolActionQueries<'w, 's> {
    orbit: Query<'w, 's, &'static Action<OrbitCamOrbitEngagedAction>>,
    pan:   Query<'w, 's, &'static Action<OrbitCamPanEngagedAction>>,
    zoom:  Query<'w, 's, &'static Action<OrbitCamZoomEngagedAction>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeldEngagement {
    Engaged,
    Idle,
}

impl From<bool> for HeldEngagement {
    fn from(engaged: bool) -> Self { if engaged { Self::Engaged } else { Self::Idle } }
}

fn held_sources_for_state<'a, A: HeldCameraAction + 'a>(
    engagement: HeldEngagement,
    entries: impl IntoIterator<Item = &'a HeldActionBindingEntry<A>>,
    fallback: InteractionSources,
    inputs: &LiveInputs<'_>,
) -> InteractionSources {
    if matches!(engagement, HeldEngagement::Idle) {
        return InteractionSources::NONE;
    }

    input::attributed_sources(entries, inputs, fallback)
}

fn pan_overrides_orbit(bindings: &OrbitCamBindings, inputs: &LiveInputs<'_>) -> bool {
    bindings.pan().enabled_entries().any(|pan| {
        let Some((pan_button, pan_mod_keys)) = pan
            .engagement_descriptor()
            .enabled_mouse_button_engagement()
        else {
            return false;
        };
        if !pan.engagement_descriptor().enabled_is_active(inputs) {
            return false;
        }
        bindings.orbit().enabled_entries().any(|orbit| {
            let Some((orbit_button, orbit_mod_keys)) = orbit
                .engagement_descriptor()
                .enabled_mouse_button_engagement()
            else {
                return false;
            };
            pan_button == orbit_button
                && mod_key_count(pan_mod_keys) > mod_key_count(orbit_mod_keys)
                && orbit.engagement_descriptor().enabled_is_active(inputs)
        })
    })
}

fn mod_key_count(mod_keys: ModKeys) -> usize { mod_keys.iter_names().count() }

fn bool_action_active<A: InputAction<Output = bool>>(
    action: Entity,
    actions: &Query<&Action<A>>,
    states: &Query<&TriggerState>,
) -> bool {
    action_state_active(action, states) && actions.get(action).is_ok_and(|action| **action)
}

/// The active speed for a held kind: any contribution from the slow motion
/// action means the slow variant is engaged. The normal and slow actions are
/// mutually exclusive (BEI's gate conditions ensure only one carries a value).
fn vec2_speed(slow: Vec2) -> ControlSpeed {
    if slow == Vec2::ZERO {
        ControlSpeed::Normal
    } else {
        ControlSpeed::Slow
    }
}

fn f32_speed(slow: f32) -> ControlSpeed {
    if slow == 0.0 {
        ControlSpeed::Normal
    } else {
        ControlSpeed::Slow
    }
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
