//! Stages adapter-side custom input values from raw `bevy::input` resources.
//!
//! Each frame [`inject_adapter_actions`] reads `AccumulatedMouseScroll`,
//! `AccumulatedMouseMotion`, `PinchGesture` events, the keyboard / mouse-button state, and
//! the touch tracker, then writes [`ActionValue`]s into the camera's custom inputs
//! (`OrbitCamAdapterOrbitAction`, `OrbitCamAdapterPanAction`, the two adapter zoom actions).
//!
//! Shared adapter carrier types live in `adapter/mod.rs`; this module writes
//! [`OrbitCamAdapterFrameSources`] for `resolve.rs`.
//!
//! Types:
//! - [`AdapterContributions`] — file-local accumulator passed between the `apply_*` helpers.
//! - [`TrackpadScrollTarget`] / [`TrackpadScrollCandidate`] — trackpad-scroll dispatch enum and
//!   per-binding candidate.
//! - [`OrbitCamTouchAdapterOverride`] — test-only resource that forces the touch gesture stream.
//!
//! Helpers handle the five contribution sources (mouse wheel, trackpad scroll, pinch,
//! touch, button-drag-zoom) plus the trackpad-binding selection and modifier-suppression
//! logic for pinch.

use bevy::input::gestures::PinchGesture;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::input::mouse::MouseScrollUnit;
use bevy::prelude::ButtonInput;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::MessageReader;
use bevy::prelude::MouseButton;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
#[cfg(test)]
use bevy::prelude::Resource;
use bevy::prelude::Vec2;
use bevy_enhanced_input::prelude::ActionValue;
use bevy_enhanced_input::prelude::CustomInputs;
use bevy_enhanced_input::prelude::ModKeys;

use super::OrbitCamAdapterFrameSources;
use super::install::OrbitCamAdapterCustomInputs;
use super::install::TrackpadBindingCondition;
use crate::OrbitCamKind;
use crate::input;
use crate::input::BUTTON_ZOOM_SCALE;
use crate::input::CameraInputContextGated;
use crate::input::CameraInstalledBindings;
use crate::input::InputGain;
use crate::input::InteractionSources;
use crate::input::LiveInputs;
use crate::input::OrbitCamBindingWithInputGain;
use crate::input::OrbitCamBindings;
use crate::input::OrbitCamButtonDragZoomAxis;
use crate::input::OrbitCamInputInstallationOf;
use crate::input::OrbitCamTouchBinding;
use crate::input::OrbitCamTrackpadScroll;
use crate::input::PINCH_GESTURE_AMPLIFICATION;
use crate::input::ResolvedCameraInputRoute;
use crate::input::TOUCH_PINCH_SCALE;
use crate::input::TouchGestures;
use crate::input::TouchTracker;
use crate::input::ZoomInversion;

pub(super) fn inject_adapter_actions(
    route: Res<ResolvedCameraInputRoute>,
    scroll: Option<Res<AccumulatedMouseScroll>>,
    motion: Option<Res<AccumulatedMouseMotion>>,
    keyboard: Option<Res<ButtonInput<KeyCode>>>,
    mouse_buttons: Option<Res<ButtonInput<MouseButton>>>,
    mut pinch_events: MessageReader<PinchGesture>,
    touch_tracker: Option<Res<TouchTracker>>,
    mut cameras: Query<(
        Entity,
        &CameraInstalledBindings<OrbitCamKind>,
        &OrbitCamAdapterCustomInputs,
        Option<&CameraInputContextGated>,
        &mut OrbitCamAdapterFrameSources,
    )>,
    mut trackpad_conditions: Query<(&OrbitCamInputInstallationOf, &mut TrackpadBindingCondition)>,
    mut custom_inputs: ResMut<CustomInputs>,
    #[cfg(test)] touch_override: Option<Res<OrbitCamTouchAdapterOverride>>,
) {
    let scroll = scroll.as_deref().copied().unwrap_or_default();
    let motion = motion
        .as_deref()
        .map(|motion| motion.delta)
        .unwrap_or_default();
    let pinch = pinch_events.read().map(|event| event.0).sum::<f32>();
    let touch_gestures = {
        #[cfg(test)]
        if let Some(override_gestures) = touch_override {
            override_gestures.0.clone()
        } else if let Some(touch_tracker) = touch_tracker {
            touch_tracker.get_touch_gestures()
        } else {
            TouchGestures::None
        }
        #[cfg(not(test))]
        if let Some(touch_tracker) = touch_tracker {
            touch_tracker.get_touch_gestures()
        } else {
            TouchGestures::None
        }
    };

    for (camera, bindings, adapter_inputs, gated, mut frame_sources) in &mut cameras {
        *frame_sources = OrbitCamAdapterFrameSources::default();
        if route.routed_camera() != Some(camera)
            || route.metrics_for(camera).is_none()
            || route
                .blockers_for(camera)
                .is_some_and(crate::input::CameraInputBlockers::is_blocked)
            || gated.is_some_and(|gated| !gated.context_gate.is_allowed())
        {
            refresh_trackpad_conditions(camera, None, &mut trackpad_conditions);
            stage_adapter_inputs(
                adapter_inputs,
                AdapterContributions::default(),
                &mut custom_inputs,
            );
            continue;
        }

        let trackpad_selection =
            selected_trackpad_binding(&bindings.0, scroll, keyboard.as_deref());
        refresh_trackpad_conditions(camera, trackpad_selection, &mut trackpad_conditions);
        let contributions = adapter_contributions(
            &bindings.0,
            scroll,
            motion,
            pinch,
            &touch_gestures,
            trackpad_selection,
            keyboard.as_deref(),
            mouse_buttons.as_deref(),
        );
        *frame_sources = contributions.sources;
        stage_adapter_inputs(adapter_inputs, contributions, &mut custom_inputs);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct AdapterContributions {
    orbit:       Vec2,
    pan:         Vec2,
    trackpad:    Vec2,
    zoom_coarse: f32,
    zoom_smooth: f32,
    sources:     OrbitCamAdapterFrameSources,
}

fn adapter_contributions(
    bindings: &OrbitCamBindings,
    scroll: AccumulatedMouseScroll,
    mouse_motion: Vec2,
    pinch: f32,
    touch_gestures: &TouchGestures,
    trackpad_selection: Option<TrackpadScrollCandidate>,
    keyboard: Option<&ButtonInput<KeyCode>>,
    mouse_buttons: Option<&ButtonInput<MouseButton>>,
) -> AdapterContributions {
    let mut contributions = AdapterContributions::default();
    apply_mouse_wheel_zoom_contribution(bindings, scroll, &mut contributions);
    apply_trackpad_scroll_contribution(scroll, trackpad_selection, &mut contributions);
    apply_pinch_contribution(bindings, pinch, keyboard, mouse_buttons, &mut contributions);
    apply_touch_contribution(bindings, touch_gestures, &mut contributions);
    apply_button_drag_zoom_contribution(bindings, mouse_motion, mouse_buttons, &mut contributions);
    contributions
}

fn apply_mouse_wheel_zoom_contribution(
    bindings: &OrbitCamBindings,
    scroll: AccumulatedMouseScroll,
    contributions: &mut AdapterContributions,
) {
    let Some(mouse_wheel_zoom) = bindings.enabled_mouse_wheel_zoom() else {
        return;
    };
    if scroll.delta == Vec2::ZERO || scroll.unit != MouseScrollUnit::Line {
        return;
    }

    contributions.zoom_coarse += zoom_signed(
        scroll.delta.y * mouse_wheel_zoom.input_gain().value(),
        bindings,
    );
    contributions.sources.zoom_coarse = contributions
        .sources
        .zoom_coarse
        .union(InteractionSources::WHEEL);
}

fn apply_trackpad_scroll_contribution(
    scroll: AccumulatedMouseScroll,
    selection: Option<TrackpadScrollCandidate>,
    contributions: &mut AdapterContributions,
) {
    if scroll.delta == Vec2::ZERO || scroll.unit != MouseScrollUnit::Pixel {
        return;
    }

    let Some(selection) = selection else {
        return;
    };
    debug_assert!(selection.input_gain.is_enabled());

    contributions.trackpad = scroll.delta;
    match selection.target {
        TrackpadScrollTarget::Orbit => {
            contributions.sources.orbit = contributions
                .sources
                .orbit
                .union(InteractionSources::SMOOTH_SCROLL);
        },
        TrackpadScrollTarget::Pan => {
            contributions.sources.pan = contributions
                .sources
                .pan
                .union(InteractionSources::SMOOTH_SCROLL);
        },
        TrackpadScrollTarget::Zoom => {
            contributions.sources.zoom_smooth = contributions
                .sources
                .zoom_smooth
                .union(InteractionSources::SMOOTH_SCROLL);
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TrackpadScrollTarget {
    Orbit,
    Pan,
    Zoom,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TrackpadScrollCandidate {
    target:     TrackpadScrollTarget,
    index:      usize,
    mod_keys:   ModKeys,
    input_gain: InputGain,
}

fn selected_trackpad_binding(
    bindings: &OrbitCamBindings,
    scroll: AccumulatedMouseScroll,
    keyboard: Option<&ButtonInput<KeyCode>>,
) -> Option<TrackpadScrollCandidate> {
    if scroll.delta == Vec2::ZERO || scroll.unit != MouseScrollUnit::Pixel {
        return None;
    }
    let candidates =
        bindings
            .enabled_trackpad_orbit()
            .map(|(index, binding)| trackpad_candidate(TrackpadScrollTarget::Orbit, index, binding))
            .chain(bindings.enabled_trackpad_pan().map(|(index, binding)| {
                trackpad_candidate(TrackpadScrollTarget::Pan, index, binding)
            }))
            .chain(bindings.enabled_trackpad_zoom().map(|(index, binding)| {
                trackpad_candidate(TrackpadScrollTarget::Zoom, index, binding)
            }));

    candidates
        .filter(|candidate| trackpad_mod_keys_pressed(keyboard, candidate.mod_keys))
        .max_by_key(|candidate| {
            (
                mod_key_count(candidate.mod_keys),
                trackpad_target_priority(candidate.target),
                candidate.index,
            )
        })
}

fn refresh_trackpad_conditions(
    camera: Entity,
    selection: Option<TrackpadScrollCandidate>,
    conditions: &mut Query<(&OrbitCamInputInstallationOf, &mut TrackpadBindingCondition)>,
) {
    for (installation, mut condition) in conditions {
        if installation.camera() != camera {
            continue;
        }
        condition.active = selection.is_some_and(|selection| {
            selection.target == condition.target
                && selection.index == condition.index
                && selection.mod_keys == condition.mod_keys
        });
    }
}

const fn trackpad_candidate(
    target: TrackpadScrollTarget,
    index: usize,
    binding: OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>,
) -> TrackpadScrollCandidate {
    TrackpadScrollCandidate {
        target,
        index,
        mod_keys: binding.binding().mod_keys,
        input_gain: binding.input_gain(),
    }
}

fn trackpad_mod_keys_pressed(keyboard: Option<&ButtonInput<KeyCode>>, mod_keys: ModKeys) -> bool {
    if mod_keys.is_empty() {
        return true;
    }
    keyboard.is_some_and(|keyboard| input::mod_keys_pressed(keyboard, mod_keys))
}

fn mod_key_count(mod_keys: ModKeys) -> usize { mod_keys.iter_names().count() }

const fn trackpad_target_priority(target: TrackpadScrollTarget) -> usize {
    match target {
        TrackpadScrollTarget::Orbit => 0,
        TrackpadScrollTarget::Pan => 1,
        TrackpadScrollTarget::Zoom => 2,
    }
}

fn apply_pinch_contribution(
    bindings: &OrbitCamBindings,
    pinch: f32,
    keyboard: Option<&ButtonInput<KeyCode>>,
    mouse_buttons: Option<&ButtonInput<MouseButton>>,
    contributions: &mut AdapterContributions,
) {
    let Some(pinch_zoom) = bindings.enabled_pinch_zoom_binding() else {
        return;
    };
    if pinch == 0.0 || pinch_suppressed(bindings, keyboard, mouse_buttons) {
        return;
    }

    contributions.zoom_smooth += zoom_signed(
        pinch * PINCH_GESTURE_AMPLIFICATION * pinch_zoom.input_gain().value(),
        bindings,
    );
    contributions.sources.zoom_smooth = contributions
        .sources
        .zoom_smooth
        .union(InteractionSources::PINCH);
}

fn pinch_suppressed(
    bindings: &OrbitCamBindings,
    keyboard: Option<&ButtonInput<KeyCode>>,
    mouse_buttons: Option<&ButtonInput<MouseButton>>,
) -> bool {
    let inputs = LiveInputs {
        keyboard,
        mouse: mouse_buttons,
        gamepads: &[],
    };

    trackpad_modifier_active(bindings, keyboard)
        || bindings
            .orbit()
            .enabled_entries()
            .any(|entry| entry.engagement_descriptor().enabled_is_active(&inputs))
        || bindings
            .pan()
            .enabled_entries()
            .any(|entry| entry.engagement_descriptor().enabled_is_active(&inputs))
        || bindings
            .zoom_smooth()
            .enabled_entries()
            .any(|entry| entry.engagement_descriptor().enabled_is_active(&inputs))
}

fn trackpad_modifier_active(
    bindings: &OrbitCamBindings,
    keyboard: Option<&ButtonInput<KeyCode>>,
) -> bool {
    let Some(keyboard) = keyboard else {
        return false;
    };

    bindings
        .enabled_trackpad_orbit()
        .map(|(_, binding)| binding)
        .chain(bindings.enabled_trackpad_pan().map(|(_, binding)| binding))
        .chain(bindings.enabled_trackpad_zoom().map(|(_, binding)| binding))
        .any(|binding| {
            !binding.binding().mod_keys.is_empty()
                && input::mod_keys_pressed(keyboard, binding.binding().mod_keys)
        })
}

fn apply_touch_contribution(
    bindings: &OrbitCamBindings,
    touch_gestures: &TouchGestures,
    contributions: &mut AdapterContributions,
) {
    let Some(touch) = bindings.enabled_touch_config() else {
        return;
    };

    let (orbit, pan, zoom) = match (touch.binding(), touch_gestures) {
        (OrbitCamTouchBinding::OneFingerOrbit, TouchGestures::OneFinger(gesture)) => {
            (gesture.motion, Vec2::ZERO, 0.0)
        },
        (OrbitCamTouchBinding::OneFingerOrbit, TouchGestures::TwoFinger(gesture)) => (
            Vec2::ZERO,
            gesture.motion,
            gesture.pinch * TOUCH_PINCH_SCALE,
        ),
        (OrbitCamTouchBinding::TwoFingerOrbit, TouchGestures::OneFinger(gesture)) => {
            (Vec2::ZERO, gesture.motion, 0.0)
        },
        (OrbitCamTouchBinding::TwoFingerOrbit, TouchGestures::TwoFinger(gesture)) => (
            gesture.motion,
            Vec2::ZERO,
            gesture.pinch * TOUCH_PINCH_SCALE,
        ),
        (_, TouchGestures::None) => (Vec2::ZERO, Vec2::ZERO, 0.0),
    };

    if touch.orbit_enabled() && orbit != Vec2::ZERO {
        let orbit = orbit * touch.input_gain().orbit_input_gain().value();
        if orbit != Vec2::ZERO {
            contributions.orbit += orbit;
            contributions.sources.orbit =
                contributions.sources.orbit.union(InteractionSources::TOUCH);
        }
    }
    if touch.pan_enabled() && pan != Vec2::ZERO {
        let pan = pan * touch.input_gain().pan_input_gain().value();
        if pan != Vec2::ZERO {
            contributions.pan += pan;
            contributions.sources.pan = contributions.sources.pan.union(InteractionSources::TOUCH);
        }
    }
    if touch.zoom_enabled() && zoom != 0.0 {
        let zoom = zoom * touch.input_gain().zoom_input_gain().value();
        if zoom != 0.0 {
            contributions.zoom_smooth += zoom_signed(zoom, bindings);
            contributions.sources.zoom_smooth = contributions
                .sources
                .zoom_smooth
                .union(InteractionSources::TOUCH);
        }
    }
}

fn apply_button_drag_zoom_contribution(
    bindings: &OrbitCamBindings,
    mouse_motion: Vec2,
    mouse_buttons: Option<&ButtonInput<MouseButton>>,
    contributions: &mut AdapterContributions,
) {
    let Some(button_drag_zoom) = bindings.enabled_button_drag_zoom() else {
        return;
    };
    if mouse_motion == Vec2::ZERO
        || mouse_buttons.is_none_or(|buttons| !buttons.pressed(button_drag_zoom.binding().button))
    {
        return;
    }

    let delta = match button_drag_zoom.binding().axis {
        OrbitCamButtonDragZoomAxis::X => mouse_motion.x,
        OrbitCamButtonDragZoomAxis::Y => -mouse_motion.y,
        OrbitCamButtonDragZoomAxis::XY => mouse_motion.x - mouse_motion.y,
    };

    contributions.zoom_smooth += zoom_signed(
        delta * BUTTON_ZOOM_SCALE * button_drag_zoom.input_gain().value(),
        bindings,
    );
    contributions.sources.zoom_smooth = contributions
        .sources
        .zoom_smooth
        .union(InteractionSources::MOUSE);
}

fn zoom_signed(value: f32, bindings: &OrbitCamBindings) -> f32 {
    let zoom_sign = match bindings.zoom_inversion() {
        ZoomInversion::Normal => 1.0,
        ZoomInversion::Inverted => -1.0,
    };
    value * zoom_sign
}

fn stage_adapter_inputs(
    inputs: &OrbitCamAdapterCustomInputs,
    contributions: AdapterContributions,
    custom_inputs: &mut CustomInputs,
) {
    custom_inputs.insert(inputs.orbit, ActionValue::Axis2D(contributions.orbit));
    custom_inputs.insert(inputs.pan, ActionValue::Axis2D(contributions.pan));
    custom_inputs.insert(inputs.trackpad, ActionValue::Axis2D(contributions.trackpad));
    custom_inputs.insert(
        inputs.zoom_coarse,
        ActionValue::Axis1D(contributions.zoom_coarse),
    );
    custom_inputs.insert(
        inputs.zoom_smooth,
        ActionValue::Axis1D(contributions.zoom_smooth),
    );
}

#[cfg(test)]
#[derive(Resource, Clone, Debug)]
pub(super) struct OrbitCamTouchAdapterOverride(pub(super) TouchGestures);
