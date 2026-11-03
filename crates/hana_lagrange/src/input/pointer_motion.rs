//! Selects one mouse-motion source for the lifetime of a camera drag.

use std::collections::HashMap;

use bevy::camera::RenderTarget;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::CursorMoved;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use bevy_enhanced_input::prelude::ActionSources;
use bevy_enhanced_input::prelude::ActionValue;
use bevy_enhanced_input::prelude::CustomInput;
use bevy_enhanced_input::prelude::CustomInputs;

use super::CameraInstalledBindings;
use super::HeldActionBindingEntry;
use super::HeldCameraAction;
use super::ResolvedCameraInputRoute;
use super::install::CameraMotionSourceCondition;
use crate::FreeCamKind;
use crate::OrbitCamKind;

/// Motion source chosen for the lifetime of one camera drag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum DragMotionSource {
    #[default]
    Undecided,
    RawMotion,
    CursorDelta,
}

/// Motion delivered to adapter actions after the routed camera chooses its source.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub(crate) struct RoutedDragMotion {
    pub(crate) delta: Vec2,
    source:           DragMotionSource,
}

#[derive(Resource)]
pub(super) struct CursorDragInput(pub(super) CustomInput);

#[derive(Resource, Default)]
pub(super) struct StagedCursorMotion(HashMap<Entity, Vec2>);

#[derive(Default)]
enum DragOwner {
    #[default]
    Idle,
    Active {
        camera:          Entity,
        source:          DragMotionSource,
        pending_cursor:  Vec2,
        waiting_for_raw: bool,
    },
}

#[derive(Resource, Default)]
pub(super) struct DragMotionLatch(DragOwner);

/// Accumulates each window's logical cursor movement in physical pixels.
pub(super) fn stage_cursor_motion(
    mut moved: MessageReader<CursorMoved>,
    windows: Query<&Window>,
    mut staged: ResMut<StagedCursorMotion>,
) {
    staged.0.clear();
    for event in moved.read() {
        if let (Some(delta), Ok(window)) = (event.delta, windows.get(event.window)) {
            *staged.0.entry(event.window).or_default() += delta * window.scale_factor();
        }
    }
}

/// Chooses raw device motion or the routed window's cursor delta for this drag.
pub(super) fn select_drag_motion(
    route: Res<ResolvedCameraInputRoute>,
    staged: Res<StagedCursorMotion>,
    raw: Option<Res<AccumulatedMouseMotion>>,
    action_sources: Res<ActionSources>,
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    cameras: Query<&RenderTarget>,
    windows: Query<(Entity, Has<PrimaryWindow>), With<Window>>,
    orbit: Query<&CameraInstalledBindings<OrbitCamKind>>,
    free: Query<&CameraInstalledBindings<FreeCamKind>>,
    mut latch: ResMut<DragMotionLatch>,
    mut selected: ResMut<RoutedDragMotion>,
    cursor_input: Res<CursorDragInput>,
    mut custom_inputs: ResMut<CustomInputs>,
    mut conditions: Query<&mut CameraMotionSourceCondition>,
) {
    let raw_delta = raw.map_or(Vec2::ZERO, |motion| motion.delta);
    let camera = route.routed_camera();
    let cursor_delta = camera
        .and_then(|camera| cameras.get(camera).ok())
        .and_then(|target| match target {
            RenderTarget::Window(WindowRef::Entity(window)) => Some(*window),
            RenderTarget::Window(WindowRef::Primary) => windows
                .iter()
                .find_map(|(entity, primary)| primary.then_some(entity)),
            _ => None,
        })
        .and_then(|window| staged.0.get(&window).copied())
        .unwrap_or_default();
    let drag_pressed = |camera| {
        buttons.as_deref().is_some_and(|buttons| {
            orbit.get(camera).is_ok_and(|installed| {
                let bindings = &installed.0;
                held_drag_pressed(bindings.orbit().enabled_entries(), buttons)
                    || held_drag_pressed(bindings.pan().enabled_entries(), buttons)
                    || held_drag_pressed(bindings.zoom_smooth().enabled_entries(), buttons)
                    || bindings.button_drag_zoom().is_some_and(|zoom| {
                        zoom.input_gain().is_enabled() && buttons.pressed(zoom.binding().button)
                    })
            }) || free.get(camera).is_ok_and(|installed| {
                let bindings = &installed.0;
                held_drag_pressed(bindings.translate().enabled_entries(), buttons)
                    || held_drag_pressed(bindings.look().enabled_entries(), buttons)
                    || held_drag_pressed(bindings.roll().enabled_entries(), buttons)
            })
        })
    };

    if let DragOwner::Active { camera: owner, .. } = &latch.0
        && !drag_pressed(*owner)
    {
        latch.0 = DragOwner::Idle;
    }
    if let Some(camera) = camera.filter(|&camera| drag_pressed(camera))
        && !matches!(&latch.0, DragOwner::Active { camera: owner, .. } if *owner == camera)
    {
        latch.0 = DragOwner::Active {
            camera,
            source: DragMotionSource::Undecided,
            pending_cursor: Vec2::ZERO,
            waiting_for_raw: false,
        };
    }

    let mut frame = RoutedDragMotion::default();
    if let DragOwner::Active {
        camera: owner,
        source,
        pending_cursor,
        waiting_for_raw,
        ..
    } = &mut latch.0
        && camera == Some(*owner)
    {
        match source {
            DragMotionSource::RawMotion => frame.delta = raw_delta,
            DragMotionSource::CursorDelta => frame.delta = cursor_delta,
            DragMotionSource::Undecided if raw_delta != Vec2::ZERO => {
                *source = DragMotionSource::RawMotion;
                frame.delta = raw_delta;
            },
            DragMotionSource::Undecided if cursor_delta != Vec2::ZERO && !*waiting_for_raw => {
                // A local mouse may report its cursor movement one update before
                // winit reports device motion. Keep this delta until the next update
                // that carries motion.
                *pending_cursor = cursor_delta;
                *waiting_for_raw = true;
            },
            DragMotionSource::Undecided if *waiting_for_raw && cursor_delta != Vec2::ZERO => {
                *source = DragMotionSource::CursorDelta;
                frame.delta = *pending_cursor + cursor_delta;
            },
            DragMotionSource::Undecided => {},
        }
        frame.source = *source;
    } else if matches!(latch.0, DragOwner::Idle) {
        // Preserve standalone mouse-motion bindings that have no drag button.
        frame.delta = raw_delta;
        frame.source = DragMotionSource::RawMotion;
    }
    if !action_sources.mouse_motion {
        frame.delta = Vec2::ZERO;
    }
    *selected = frame;
    let cursor_value = if frame.source == DragMotionSource::CursorDelta {
        frame.delta
    } else {
        Vec2::ZERO
    };
    custom_inputs.insert(cursor_input.0, ActionValue::Axis2D(cursor_value));
    for mut condition in &mut conditions {
        let camera_is_routed = camera == Some(condition.camera());
        condition.select(camera_is_routed, frame.source);
    }
}

fn held_drag_pressed<'a, A: HeldCameraAction + 'a>(
    mut entries: impl Iterator<Item = &'a HeldActionBindingEntry<A>>,
    buttons: &ButtonInput<MouseButton>,
) -> bool {
    entries.any(|entry| {
        entry.motion_descriptor().enabled_entries().any(|motion| {
            matches!(
                motion.binding(),
                bevy_enhanced_input::prelude::Binding::MouseMotion { .. }
            )
        }) && entry
            .engagement_descriptor()
            .mouse_button_engagement()
            .is_some_and(|(button, _)| buttons.pressed(button))
    })
}
