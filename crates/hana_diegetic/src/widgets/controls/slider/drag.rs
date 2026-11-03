use std::collections::HashMap;
use std::collections::HashSet;

use bevy::camera::NormalizedRenderTarget;
use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::query::QueryData;
use bevy::ecs::world::DeferredWorld;
use bevy::picking::pointer::PointerAction;
use bevy::picking::pointer::PointerId;
use bevy::picking::pointer::PointerInput;
use bevy::prelude::Add;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Despawn;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Has;
use bevy::prelude::MessageReader;
use bevy::prelude::On;
use bevy::prelude::PointerButton;
use bevy::prelude::PointerCancel;
use bevy::prelude::PointerClick;
use bevy::prelude::PointerDrag;
use bevy::prelude::PointerDragEnd;
use bevy::prelude::PointerPress;
use bevy::prelude::PointerRelease;
use bevy::prelude::Query;
use bevy::prelude::Remove;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Vec2;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::prelude::warn;

use super::SliderAuthoringLock;
use super::SliderChangeRequested;
use super::SliderProjection;
use super::SliderProposalSource;
use super::SliderProposalValue;
use super::SliderState;
use super::adjustment;
use super::projection;
use super::projection::ProjectionUnavailable;
use super::projection::SliderPointerPosition;
use super::projection::SlotTranslationUnavailable;
use crate::FineAdjustment;
use crate::PanelElementId;
use crate::constants::DIAGNOSTIC_LOG_TARGET;
use crate::layout::BoundingBox;
use crate::widgets::PanelWidget;
use crate::widgets::SemanticWidgetIntent;
use crate::widgets::VisualSlotId;
use crate::widgets::WidgetDisabled;
use crate::widgets::WidgetKind;
use crate::widgets::WidgetOf;
use crate::widgets::WidgetSpec;
use crate::widgets::WidgetVisualSlots;
use crate::widgets::constants::THUMB_CLICK_JITTER_PIXELS_SQUARED;
use crate::widgets::constants::THUMB_HIT_SLOP_POINTS;
use crate::widgets::interaction as capture;
use crate::widgets::interaction::WidgetCaptures;

/// Reports the beginning of a pointer-driven slider drag.
#[derive(Clone, Debug, EntityEvent)]
pub struct SliderGrabbed {
    /// Live slider entity the pointer grabbed.
    #[event_target]
    pub entity:     Entity,
    /// Panel-local slider id.
    pub id:         PanelElementId,
    /// Pointer that began the drag.
    pub pointer_id: PointerId,
}

/// Reports the valid release of a pointer-driven slider drag.
///
/// The matching final [`SliderChangeRequested`] proposal is emitted first,
/// after the pointer's shared occupancy is freed.
#[derive(Clone, Debug, EntityEvent)]
pub struct SliderReleased {
    /// Live slider entity the pointer released.
    #[event_target]
    pub entity:     Entity,
    /// Panel-local slider id.
    pub id:         PanelElementId,
    /// Pointer that completed the drag.
    pub pointer_id: PointerId,
}

/// Reports a pointer-driven slider drag that ended without a valid release.
#[derive(Clone, Debug, EntityEvent)]
pub struct SliderCanceled {
    /// Live slider entity whose drag was canceled.
    #[event_target]
    pub entity:     Entity,
    /// Panel-local slider id.
    pub id:         PanelElementId,
    /// Pointer whose drag was canceled.
    pub pointer_id: PointerId,
    /// Reason the drag ended without a valid release.
    pub cause:      SliderCancelCause,
}

/// Reason a pointer-driven slider drag was canceled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SliderCancelCause {
    /// The pointer input stream reported cancellation.
    PointerCanceled,
    /// The captured pointer entity or its [`PointerId`] was removed.
    PointerRemoved,
    /// The captured pointer released without an active drag to complete.
    CaptureLost,
    /// The slider became disabled while dragging.
    Disabled,
    /// Cable replacement locked this slider while dragging.
    AuthoringLocked,
    /// A drag or release position could no longer project onto the slider.
    ProjectionUnavailable,
    /// The widget was removed or its owning panel role ended.
    WidgetRemoved,
    /// The same widget id changed to another widget kind.
    WidgetKindChanged,
    /// Semantic input explicitly canceled the drag.
    Explicit,
}

/// Live marker for a captured slider drag.
///
/// Removing or despawning it runs [`emit_slider_terminal`], which frees the
/// shared occupancy and emits exactly one terminal event from the recorded
/// [`SliderTerminal`] outcome.
#[derive(Component)]
#[component(
    on_remove = emit_slider_terminal,
    on_despawn = emit_slider_terminal
)]
pub(crate) struct SliderDrag;

/// Recorded terminal outcome for one captured slider drag.
#[derive(Clone, Copy)]
enum SliderTerminal {
    /// The drag is live; no terminal has been requested.
    Pending,
    /// A valid release reprojected the given raw-domain value.
    Release(SliderProposalValue),
    /// The drag was canceled for the given reason.
    Cancel(SliderCancelCause),
}

/// Release behavior selected when a pointer press begins.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ClickResolution {
    Ordinary,
    ResetDefault,
}

/// Where a pointer press began and whether it has left click-jitter tolerance.
///
/// Only a thumb press still inside [`THUMB_CLICK_JITTER_PIXELS_SQUARED`] can
/// resolve as [`ClickResolution::ResetDefault`], so the resolution is stored in
/// [`PressPhase::ThumbHeld`] rather than beside the origin as a separate field.
#[derive(Clone, Copy, Eq, PartialEq)]
enum PressPhase {
    /// The press landed on the track, so every drag position updates the raw
    /// target.
    Track,
    /// The press landed on the marked thumb and has not moved beyond
    /// [`THUMB_CLICK_JITTER_PIXELS_SQUARED`], so the applied value is held.
    ThumbHeld(ClickResolution),
    /// The press landed on the marked thumb and left that tolerance, so drag
    /// positions update the raw target and no click resolution survives.
    ThumbDragging,
}

/// The facts one slider press records, shared by a live drag and by a press
/// awaiting recapture.
struct SliderPress {
    id:              PanelElementId,
    camera:          Entity,
    captured_target: NormalizedRenderTarget,
    position:        Vec2,
    value:           SliderProposalValue,
    pointer_value:   SliderPointerPosition,
    adjustment:      crate::FineAdjustment,
    range:           super::SliderRange,
    phase:           PressPhase,
}

/// Slider-only capture payload keyed by the captured widget entity.
///
/// Pointer/widget occupancy and raw-action ordering live in the shared
/// [`WidgetCaptures`]; this holds only the slider facts terminal emission and
/// reprojection need: the originating [`SliderPress`], the owning pointer, the
/// latest raw projected target, and the typed terminal outcome.
struct SliderCapture {
    press:             SliderPress,
    pointer_id:        PointerId,
    latest_raw_target: SliderProposalValue,
    terminal:          SliderTerminal,
    motion:            SliderMotion,
}

/// Absolute tracking becomes relative after precision is engaged, so releasing
/// the modifier cannot pull the thumb back to the pointer.
#[derive(Clone, Copy)]
enum SliderMotion {
    Absolute { pointer_value: f32 },
    Relative { pointer_value: f32 },
}

/// A modifier flip anchors every live capture before this frame's pointer
/// events. The current proposal becomes the base for subsequent relative travel.
pub(super) fn reanchor_precision(
    adjustment: Res<crate::FineAdjustment>,
    mut captures: ResMut<SliderCaptures>,
) {
    for capture in captures.drags.values_mut() {
        if capture.press.adjustment == *adjustment {
            continue;
        }
        let pointer_value = match capture.motion {
            SliderMotion::Absolute { pointer_value } | SliderMotion::Relative { pointer_value } => {
                pointer_value
            },
        };
        capture.motion = SliderMotion::Relative { pointer_value };
        capture.press.adjustment = *adjustment;
    }
}

/// Middle takes the pointer before picking can deliver further slider motion.
pub(super) fn preempt_for_orbit(
    mut inputs: MessageReader<PointerInput>,
    mut captures: ResMut<SliderCaptures>,
    mut commands: Commands,
) {
    for input in inputs.read() {
        match input.action {
            PointerAction::Press(PointerButton::Middle) => {
                captures.orbiting_pointers.insert(input.pointer_id);
                let entities = captures
                    .drags
                    .iter()
                    .filter_map(|(&entity, capture)| {
                        (capture.pointer_id == input.pointer_id).then_some(entity)
                    })
                    .collect::<Vec<_>>();
                for entity in entities {
                    cancel_slider_drag(
                        entity,
                        SliderCancelCause::CaptureLost,
                        &mut captures,
                        &mut commands,
                    );
                }
                captures.pending.remove(&input.pointer_id);
            },
            PointerAction::Release(PointerButton::Middle) | PointerAction::Cancel => {
                captures.orbiting_pointers.remove(&input.pointer_id);
            },
            _ => {},
        }
    }
}

/// A projected slider press that shared occupancy rejected, awaiting recapture
/// by the raw dispatcher once its pointer and widget free within the batch.
struct PendingSliderPress {
    entity:   Entity,
    sequence: u64,
    press:    SliderPress,
}

/// Slider drag payloads and rejected-press pending records.
#[derive(Default, Resource)]
pub(crate) struct SliderCaptures {
    drags:             HashMap<Entity, SliderCapture>,
    pending:           HashMap<PointerId, PendingSliderPress>,
    reset_candidates:  HashMap<PointerId, Entity>,
    orbiting_pointers: HashSet<PointerId>,
}

impl SliderCaptures {
    fn has_drag_or_pending_press(&self, entity: Entity) -> bool {
        self.drags.contains_key(&entity)
            || self.pending.values().any(|press| press.entity == entity)
    }

    fn begin(&mut self, entity: Entity, pointer_id: PointerId, press: SliderPress) {
        self.drags.insert(
            entity,
            SliderCapture {
                motion: if press.adjustment == FineAdjustment::Fine {
                    SliderMotion::Relative {
                        pointer_value: press.pointer_value.fraction,
                    }
                } else {
                    SliderMotion::Absolute {
                        pointer_value: press.pointer_value.fraction,
                    }
                },
                latest_raw_target: press.value,
                press,
                pointer_id,
                terminal: SliderTerminal::Pending,
            },
        );
    }

    /// Returns the captured camera entity and normalized render target for a
    /// live drag, cloned so reprojection can borrow this resource mutably next.
    fn capture_context(&self, entity: Entity) -> Option<(Entity, NormalizedRenderTarget)> {
        self.drags
            .get(&entity)
            .map(|capture| (capture.press.camera, capture.press.captured_target.clone()))
    }

    fn motion_target(
        &mut self,
        entity: Entity,
        pointer_value: SliderPointerPosition,
        adjustment: crate::FineAdjustment,
    ) -> SliderProposalValue {
        let Some(capture) = self.drags.get_mut(&entity) else {
            return pointer_value.clamped_value();
        };
        let previous = match capture.motion {
            SliderMotion::Absolute { pointer_value } | SliderMotion::Relative { pointer_value } => {
                pointer_value
            },
        };
        if adjustment == FineAdjustment::Fine
            || matches!(capture.motion, SliderMotion::Relative { .. })
        {
            capture.motion = SliderMotion::Relative {
                pointer_value: pointer_value.fraction,
            };
            let delta = (pointer_value.fraction - previous) * adjustment.rate();
            if delta == 0.0 {
                return capture.latest_raw_target;
            }
            SliderProposalValue::snapping(
                capture
                    .press
                    .range
                    .offset_value_by_fraction(capture.latest_raw_target.get(), delta),
            )
        } else {
            capture.motion = SliderMotion::Absolute {
                pointer_value: pointer_value.fraction,
            };
            pointer_value.clamped_value()
        }
    }

    fn update_raw_target(
        &mut self,
        entity: Entity,
        position: Vec2,
        value: SliderProposalValue,
    ) -> bool {
        let pointer_id = self.drags.get_mut(&entity).and_then(|capture| {
            if let PressPhase::ThumbHeld(_) = capture.press.phase {
                if position.distance_squared(capture.press.position)
                    <= THUMB_CLICK_JITTER_PIXELS_SQUARED
                {
                    return None;
                }
                capture.press.phase = PressPhase::ThumbDragging;
            }
            capture.latest_raw_target = value;
            Some(capture.pointer_id)
        });
        if let Some(pointer_id) = pointer_id {
            self.reset_candidates.remove(&pointer_id);
            true
        } else {
            false
        }
    }

    /// Records `phase`'s press for double-click reset tracking and returns it
    /// with its [`ClickResolution`] resolved: a repeat thumb press on the same
    /// pointer and widget resolves to [`ClickResolution::ResetDefault`].
    fn resolve_press_phase(
        &mut self,
        pointer_id: PointerId,
        entity: Entity,
        count: u8,
        phase: PressPhase,
    ) -> PressPhase {
        let PressPhase::ThumbHeld(_) = phase else {
            self.reset_candidates.remove(&pointer_id);
            return PressPhase::Track;
        };
        let resets = count >= 2 && self.reset_candidates.get(&pointer_id) == Some(&entity);
        self.reset_candidates.insert(pointer_id, entity);
        PressPhase::ThumbHeld(if resets {
            ClickResolution::ResetDefault
        } else {
            ClickResolution::Ordinary
        })
    }

    fn preserves_click_value(&self, entity: Entity) -> Option<SliderProposalValue> {
        let capture = self.drags.get(&entity)?;
        matches!(capture.press.phase, PressPhase::ThumbHeld(_)).then_some(capture.latest_raw_target)
    }

    fn resets_on_click(&self, entity: Entity) -> bool {
        self.drags.get(&entity).is_some_and(|capture| {
            matches!(
                capture.press.phase,
                PressPhase::ThumbHeld(ClickResolution::ResetDefault)
            )
        })
    }

    fn clear_reset_pointer(&mut self, pointer_id: PointerId) {
        self.reset_candidates.remove(&pointer_id);
    }

    pub(super) fn clear_reset_widget(&mut self, entity: Entity) {
        self.reset_candidates
            .retain(|_, candidate| *candidate != entity);
    }

    /// Records a valid release outcome when the drag is still pending, storing
    /// the reprojected value as both the terminal payload and latest raw
    /// target. Returns whether the outcome was recorded.
    fn set_release(&mut self, entity: Entity, value: SliderProposalValue) -> bool {
        match self.drags.get_mut(&entity) {
            Some(capture) if matches!(capture.terminal, SliderTerminal::Pending) => {
                capture.terminal = SliderTerminal::Release(value);
                capture.latest_raw_target = value;
                true
            },
            _ => false,
        }
    }

    /// Records a cancellation when the drag is still pending. Returns whether
    /// the outcome was recorded.
    fn cancel(&mut self, entity: Entity, cause: SliderCancelCause) -> bool {
        let recorded = match self.drags.get_mut(&entity) {
            Some(capture) if matches!(capture.terminal, SliderTerminal::Pending) => {
                capture.terminal = SliderTerminal::Cancel(cause);
                Some(capture.pointer_id)
            },
            _ => None,
        };
        if let Some(pointer_id) = recorded {
            self.reset_candidates.remove(&pointer_id);
            true
        } else {
            false
        }
    }

    fn take(&mut self, entity: Entity) -> Option<SliderCapture> { self.drags.remove(&entity) }

    fn record_pending(&mut self, pointer_id: PointerId, pending: PendingSliderPress) {
        self.pending.insert(pointer_id, pending);
    }

    fn take_pending(&mut self, pointer_id: PointerId) -> Option<PendingSliderPress> {
        self.pending.remove(&pointer_id)
    }

    pub(crate) fn has_pending(&self) -> bool { !self.pending.is_empty() }

    pub(crate) fn clear_pending(&mut self) { self.pending.clear(); }

    #[cfg(test)]
    pub(crate) fn latest_raw_target(&self, entity: Entity) -> Option<f32> {
        self.drags
            .get(&entity)
            .map(|capture| capture.latest_raw_target.get())
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool { self.drags.is_empty() && self.pending.is_empty() }
}

/// The live slider facts pointer handling reads from one widget entity.
#[derive(QueryData)]
pub(in crate::widgets) struct SliderWidget {
    widget: &'static PanelWidget,
    kind:   &'static WidgetKind,
    owner:  &'static WidgetOf,
    state:  &'static SliderState,
    slots:  &'static WidgetVisualSlots,
    locked: Has<SliderAuthoringLock>,
}

/// Presented thumb bounds in panel layout coordinates, expanded by a small
/// pointer allowance on every side.
fn current_thumb_hit_bounds(
    slots: &WidgetVisualSlots,
    state: &SliderState,
) -> Result<BoundingBox, SlotTranslationUnavailable> {
    let slot = if slots.border_box(VisualSlotId::SLIDER_THUMB_HIT).is_some() {
        VisualSlotId::SLIDER_THUMB_HIT
    } else {
        VisualSlotId::SLIDER_THUMB
    };
    let mut bounds = slots
        .border_box(slot)
        .ok_or(SlotTranslationUnavailable::Unmarked)?;
    let translation = projection::slot_translation(slots, state, slot, state.value())?;
    bounds.x += translation.x - THUMB_HIT_SLOP_POINTS;
    bounds.y += translation.y - THUMB_HIT_SLOP_POINTS;
    bounds.width = THUMB_HIT_SLOP_POINTS.mul_add(2.0, bounds.width);
    bounds.height = THUMB_HIT_SLOP_POINTS.mul_add(2.0, bounds.height);
    Ok(bounds)
}

fn begin_drag(
    slider_captures: &mut SliderCaptures,
    commands: &mut Commands<'_, '_>,
    entity: Entity,
    pointer_id: PointerId,
    press: SliderPress,
) {
    let id = press.id.clone();
    let value = press.value;
    slider_captures.begin(entity, pointer_id, press);
    commands.entity(entity).insert(SliderDrag);
    commands.trigger(SliderGrabbed {
        entity,
        id: id.clone(),
        pointer_id,
    });
    commands.trigger(SliderChangeRequested {
        entity,
        id,
        value,
        is_final: false,
        source: SliderProposalSource::Pointer(pointer_id),
    });
}

/// Projects a slider press and, on success, claims occupancy and begins the
/// drag; a rejected but projected press is recorded for raw recapture.
pub(super) fn grab_from_pointer(
    mut press: On<PointerPress>,
    widgets: Query<
        (
            &PanelWidget,
            &WidgetKind,
            &WidgetOf,
            &SliderState,
            &WidgetVisualSlots,
            Has<WidgetDisabled>,
            Has<SliderDrag>,
        ),
        With<WidgetOf>,
    >,
    projection: SliderProjection,
    mut captures: ResMut<WidgetCaptures>,
    mut slider_captures: ResMut<SliderCaptures>,
    locks: Query<(), With<SliderAuthoringLock>>,
    mut commands: Commands,
) {
    if press.button != PointerButton::Primary
        || slider_captures
            .orbiting_pointers
            .contains(&press.pointer.id)
    {
        return;
    }
    let entity = press.event_target();
    let Ok((widget, kind, widget_of, state, slots, disabled, dragging)) = widgets.get(entity)
    else {
        return;
    };
    if *kind != WidgetKind::Slider {
        return;
    }
    press.propagate(false);
    if disabled || locks.contains(entity) {
        return;
    }
    let camera = press.hit.camera;
    let captured_target = press.pointer.target.clone();
    let viewport_position = press.pointer.position;
    let Ok((pointer_local, pointer_value)) = projection::project_widget_motion(
        &projection,
        camera,
        &captured_target,
        viewport_position,
        widget_of.panel(),
        state,
        slots,
    ) else {
        return;
    };
    let landed_on_thumb =
        current_thumb_hit_bounds(slots, state).is_ok_and(|thumb| thumb.contains(pointer_local));
    let phase = slider_captures.resolve_press_phase(
        press.pointer.id,
        entity,
        press.count,
        if landed_on_thumb {
            PressPhase::ThumbHeld(ClickResolution::Ordinary)
        } else {
            PressPhase::Track
        },
    );
    let press_record = SliderPress {
        id: widget.id().clone(),
        camera,
        captured_target,
        position: viewport_position,
        value: if landed_on_thumb || *projection.fine_adjustment == FineAdjustment::Fine {
            SliderProposalValue::preserving(state.value())
        } else {
            pointer_value.clamped_value()
        },
        pointer_value,
        adjustment: *projection.fine_adjustment,
        range: state.range(),
        phase,
    };
    let Some(sequence) = captures.observe_press(press.pointer.id, entity) else {
        return;
    };
    if !dragging && captures.try_capture(press.pointer.id, entity, sequence) {
        begin_drag(
            &mut slider_captures,
            &mut commands,
            entity,
            press.pointer.id,
            press_record,
        );
    } else {
        slider_captures.record_pending(
            press.pointer.id,
            PendingSliderPress {
                entity,
                sequence,
                press: press_record,
            },
        );
    }
}

/// Reprojects each drag and proposes the new raw target, canceling the drag
/// when the drag position no longer projects.
pub(super) fn drag_from_pointer(
    mut drag: On<PointerDrag>,
    widgets: Query<SliderWidget>,
    projection: SliderProjection,
    captures: Res<WidgetCaptures>,
    mut slider_captures: ResMut<SliderCaptures>,
    mut commands: Commands,
) {
    if drag.button != PointerButton::Primary {
        return;
    }
    let entity = drag.event_target();
    let Ok(slider) = widgets.get(entity) else {
        return;
    };
    if *slider.kind != WidgetKind::Slider || slider.locked {
        return;
    }
    drag.propagate(false);
    if !captures.captures(drag.pointer.id, entity) {
        return;
    }
    let Some((camera, captured_target)) = slider_captures.capture_context(entity) else {
        return;
    };
    match projection::project_widget_motion(
        &projection,
        camera,
        &captured_target,
        drag.pointer.position,
        slider.owner.panel(),
        slider.state,
        slider.slots,
    ) {
        Ok((_, value)) => {
            let value = slider_captures.motion_target(entity, value, *projection.fine_adjustment);
            if slider_captures.update_raw_target(entity, drag.pointer.position, value) {
                commands.trigger(SliderChangeRequested {
                    entity,
                    id: slider.widget.id().clone(),
                    value,
                    is_final: false,
                    source: SliderProposalSource::Pointer(drag.pointer.id),
                });
            }
        },
        Err(ProjectionUnavailable) => {
            cancel_slider_drag(
                entity,
                SliderCancelCause::ProjectionUnavailable,
                &mut slider_captures,
                &mut commands,
            );
        },
    }
}

pub(super) fn release_from_pointer(
    mut release: On<PointerRelease>,
    widgets: Query<SliderWidget>,
    projection: SliderProjection,
    captures: Res<WidgetCaptures>,
    mut slider_captures: ResMut<SliderCaptures>,
    mut commands: Commands,
) {
    if release.button != PointerButton::Primary {
        return;
    }
    let entity = release.event_target();
    let Ok(slider) = widgets.get(entity) else {
        return;
    };
    if *slider.kind != WidgetKind::Slider || slider.locked {
        return;
    }
    release.propagate(false);
    if !captures.captures(release.pointer.id, entity) {
        return;
    }
    resolve_release(
        entity,
        release.pointer.position,
        &widgets,
        &projection,
        &mut slider_captures,
        &mut commands,
    );
}

/// Completes a captured slider interaction at its authored default when the
/// second primary click arrives.
///
/// The reset becomes the capture's release value before [`PointerRelease`]
/// is dispatched. Removing [`SliderDrag`] then emits the ordinary final
/// proposal and frees the capture, so the following release cannot overwrite
/// the reset with the pointer position.
pub(super) fn reset_from_pointer(
    mut click: On<PointerClick>,
    sliders: Query<(&WidgetKind, &WidgetSpec, Has<WidgetDisabled>), With<PanelWidget>>,
    captures: Res<WidgetCaptures>,
    mut slider_captures: ResMut<SliderCaptures>,
    locks: Query<(), With<SliderAuthoringLock>>,
    mut commands: Commands,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    let entity = click.event_target();
    let Ok((kind, authored, disabled)) = sliders.get(entity) else {
        return;
    };
    if *kind != WidgetKind::Slider {
        return;
    }
    click.propagate(false);
    let WidgetSpec::Slider(slider) = authored else {
        return;
    };
    if disabled
        || locks.contains(entity)
        || click.count < 2
        || !captures.captures(click.pointer.id, entity)
        || !slider_captures.resets_on_click(entity)
    {
        return;
    }
    if slider_captures.set_release(
        entity,
        SliderProposalValue::preserving(slider.default_value),
    ) {
        slider_captures.clear_reset_pointer(click.pointer.id);
        commands.entity(entity).remove::<SliderDrag>();
    }
}

pub(super) fn release_from_drag_end(
    mut drag_end: On<PointerDragEnd>,
    widgets: Query<SliderWidget>,
    projection: SliderProjection,
    captures: Res<WidgetCaptures>,
    mut slider_captures: ResMut<SliderCaptures>,
    mut commands: Commands,
) {
    if drag_end.button != PointerButton::Primary {
        return;
    }
    let entity = drag_end.event_target();
    let Ok(slider) = widgets.get(entity) else {
        return;
    };
    if *slider.kind != WidgetKind::Slider || slider.locked {
        return;
    }
    if !captures.captures(drag_end.pointer.id, entity) {
        return;
    }
    drag_end.propagate(false);
    resolve_release(
        entity,
        drag_end.pointer.position,
        &widgets,
        &projection,
        &mut slider_captures,
        &mut commands,
    );
}

/// Reprojects the release position, records the release or projection-loss
/// outcome, and removes [`SliderDrag`] so its hook emits the terminal. Returns
/// whether a terminal was recorded, so the shared dispatcher marks the pointer
/// freed only when typed terminal processing will release shared occupancy. A
/// query miss, wrong kind, or absent captured payload records nothing and
/// returns false.
pub(in crate::widgets) fn resolve_release(
    entity: Entity,
    viewport_position: Vec2,
    widgets: &Query<'_, '_, SliderWidget>,
    projection: &SliderProjection<'_, '_>,
    slider_captures: &mut SliderCaptures,
    commands: &mut Commands<'_, '_>,
) -> bool {
    let Ok(slider) = widgets.get(entity) else {
        return false;
    };
    if *slider.kind != WidgetKind::Slider || slider.locked {
        return false;
    }
    let Some((camera, captured_target)) = slider_captures.capture_context(entity) else {
        return false;
    };
    let recorded = if let Some(value) = slider_captures.preserves_click_value(entity) {
        slider_captures.set_release(entity, value)
    } else {
        match projection::project_widget_motion(
            projection,
            camera,
            &captured_target,
            viewport_position,
            slider.owner.panel(),
            slider.state,
            slider.slots,
        ) {
            Ok((_, value)) => {
                let value =
                    slider_captures.motion_target(entity, value, *projection.fine_adjustment);
                slider_captures.set_release(entity, value)
            },
            Err(ProjectionUnavailable) => {
                slider_captures.cancel(entity, SliderCancelCause::ProjectionUnavailable)
            },
        }
    };
    if recorded {
        commands.entity(entity).remove::<SliderDrag>();
    }
    recorded
}

pub(super) fn cancel_from_pointer(
    mut cancel: On<PointerCancel>,
    widgets: Query<&WidgetKind>,
    captures: Res<WidgetCaptures>,
    mut slider_captures: ResMut<SliderCaptures>,
    mut commands: Commands,
) {
    let entity = cancel.event_target();
    let Ok(kind) = widgets.get(entity) else {
        return;
    };
    if *kind != WidgetKind::Slider {
        return;
    }
    if captures.captures(cancel.pointer.id, entity) {
        cancel.propagate(false);
        cancel_slider_drag(
            entity,
            SliderCancelCause::PointerCanceled,
            &mut slider_captures,
            &mut commands,
        );
    }
}

pub(super) fn cancel_from_pointer_removal(
    removed: On<Remove<PointerId>>,
    pointers: Query<&PointerId>,
    captures: Res<WidgetCaptures>,
    mut slider_captures: ResMut<SliderCaptures>,
    mut commands: Commands,
) {
    let Ok(&pointer_id) = pointers.get(removed.entity) else {
        return;
    };
    slider_captures.clear_reset_pointer(pointer_id);
    let Some(widget) = captures.widget(pointer_id) else {
        return;
    };
    cancel_slider_drag(
        widget,
        SliderCancelCause::PointerRemoved,
        &mut slider_captures,
        &mut commands,
    );
}

pub(super) fn cancel_from_disabled(
    disabled: On<Add<WidgetDisabled>>,
    mut slider_captures: ResMut<SliderCaptures>,
    mut commands: Commands,
) {
    slider_captures.clear_reset_widget(disabled.entity);
    cancel_slider_drag(
        disabled.entity,
        SliderCancelCause::Disabled,
        &mut slider_captures,
        &mut commands,
    );
}

pub(super) fn cancel_from_widget_removal(
    removed: On<Remove<PanelWidget>>,
    mut slider_captures: ResMut<SliderCaptures>,
    mut commands: Commands,
) {
    slider_captures.clear_reset_widget(removed.entity);
    cancel_slider_drag(
        removed.entity,
        SliderCancelCause::WidgetRemoved,
        &mut slider_captures,
        &mut commands,
    );
}

pub(super) fn cancel_before_widget_despawn(
    despawn: On<Despawn<PanelWidget>>,
    mut slider_captures: ResMut<SliderCaptures>,
) {
    slider_captures.clear_reset_widget(despawn.entity);
    slider_captures.cancel(despawn.entity, SliderCancelCause::WidgetRemoved);
}

pub(super) fn handle_semantic_intent(
    intent: On<SemanticWidgetIntent>,
    widgets: Query<(
        &PanelWidget,
        &WidgetKind,
        &SliderState,
        Has<WidgetDisabled>,
        Has<SliderDrag>,
    )>,
    mut slider_captures: ResMut<SliderCaptures>,
    locks: Query<(), With<SliderAuthoringLock>>,
    mut commands: Commands,
) {
    let entity = intent.event_target();
    let Ok((widget, kind, state, disabled, dragging)) = widgets.get(entity) else {
        return;
    };
    if *kind != WidgetKind::Slider {
        return;
    }
    match intent.event() {
        SemanticWidgetIntent::Cancel { .. } => {
            cancel_slider_drag(
                entity,
                SliderCancelCause::Explicit,
                &mut slider_captures,
                &mut commands,
            );
        },
        SemanticWidgetIntent::Step { direction, .. } => {
            if disabled
                || locks.contains(entity)
                || dragging
                || slider_captures.has_drag_or_pending_press(entity)
            {
                return;
            }
            adjustment::propose_slider_adjustment(
                entity,
                widget,
                state,
                adjustment::step_adjustment(state, *direction),
                SliderProposalSource::Semantic,
                &mut commands,
            );
        },
        _ => {},
    }
}

/// Records a slider cancellation and, when newly recorded, removes
/// [`SliderDrag`] so its hook emits the terminal. Returns whether a terminal
/// was recorded, so the shared dispatcher marks the pointer freed only when
/// shared occupancy will be released.
pub(in crate::widgets) fn cancel_slider_drag(
    entity: Entity,
    cause: SliderCancelCause,
    slider_captures: &mut SliderCaptures,
    commands: &mut Commands<'_, '_>,
) -> bool {
    let canceled = slider_captures.cancel(entity, cause);
    if canceled {
        commands.entity(entity).remove::<SliderDrag>();
    }
    canceled
}

/// Cancels every active slider drag owned by `panel`, mirroring
/// [`finalize_panel_buttons`](crate::widgets::finalize_panel_buttons) for teardown.
pub(crate) fn finalize_panel_sliders(
    panel: Entity,
    slider_drags: &Query<'_, '_, (Entity, &WidgetOf), With<SliderDrag>>,
    slider_captures: &mut SliderCaptures,
    commands: &mut Commands<'_, '_>,
) {
    for (entity, widget_of) in slider_drags {
        if widget_of.panel() == panel
            && slider_captures.cancel(entity, SliderCancelCause::WidgetRemoved)
        {
            commands.entity(entity).remove::<SliderDrag>();
        }
    }
}

/// Recaptures a slider press the raw dispatcher freed within the same batch,
/// using the projection stored when its initial press was rejected.
pub(in crate::widgets) fn capture_reconciled_slider_press(
    world: &mut World,
    entity: Entity,
    pointer_id: PointerId,
    sequence: u64,
) {
    let is_slider = world
        .get::<WidgetKind>(entity)
        .is_some_and(|kind| *kind == WidgetKind::Slider);
    let Some(pending) = world
        .resource_mut::<SliderCaptures>()
        .take_pending(pointer_id)
    else {
        return;
    };
    if !is_slider
        || pending.entity != entity
        || pending.sequence != sequence
        || world.get::<WidgetOf>(entity).is_none()
        || world.get::<WidgetDisabled>(entity).is_some()
        || world.get::<SliderAuthoringLock>(entity).is_some()
        || world.get::<SliderDrag>(entity).is_some()
    {
        return;
    }
    if !world
        .resource_mut::<WidgetCaptures>()
        .try_capture(pointer_id, entity, sequence)
    {
        return;
    }
    let PendingSliderPress { press, .. } = pending;
    let id = press.id.clone();
    let value = press.value;
    world
        .resource_mut::<SliderCaptures>()
        .begin(entity, pointer_id, press);
    world.entity_mut(entity).insert(SliderDrag);
    world.trigger(SliderGrabbed {
        entity,
        id: id.clone(),
        pointer_id,
    });
    world.trigger(SliderChangeRequested {
        entity,
        id,
        value,
        is_final: false,
        source: SliderProposalSource::Pointer(pointer_id),
    });
}

/// Frees shared occupancy and emits one terminal event for a removed drag.
fn emit_slider_terminal(mut world: DeferredWorld, context: HookContext) {
    let entity = context.entity;
    let Some(capture) = world
        .get_resource_mut::<SliderCaptures>()
        .and_then(|mut captures| captures.take(entity))
    else {
        return;
    };
    let Some(pointer_id) = world
        .get_resource_mut::<WidgetCaptures>()
        .and_then(|mut captures| captures.release_widget(entity))
    else {
        warn!(
            target: DIAGNOSTIC_LOG_TARGET, "slider {:?} ({entity}) has an active drag but no capture owner; skipping terminal \
             events",
            capture.press.id
        );
        return;
    };
    let SliderCapture {
        press, terminal, ..
    } = capture;
    let id = press.id;
    match terminal {
        SliderTerminal::Release(value) => {
            capture::trigger_immediate(
                &mut world,
                SliderChangeRequested {
                    entity,
                    id: id.clone(),
                    value,
                    is_final: true,
                    source: SliderProposalSource::Pointer(pointer_id),
                },
            );
            capture::trigger_immediate(
                &mut world,
                SliderReleased {
                    entity,
                    id,
                    pointer_id,
                },
            );
        },
        SliderTerminal::Cancel(cause) => {
            capture::trigger_immediate(
                &mut world,
                SliderCanceled {
                    entity,
                    id,
                    pointer_id,
                    cause,
                },
            );
        },
        SliderTerminal::Pending => {
            capture::trigger_immediate(
                &mut world,
                SliderCanceled {
                    entity,
                    id,
                    pointer_id,
                    cause: SliderCancelCause::CaptureLost,
                },
            );
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::camera::PerspectiveProjection;
    use bevy::camera::Projection;
    use bevy::camera::RenderTarget;
    use bevy::camera::RenderTargetInfo;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::ButtonState;
    use bevy::input::InputPlugin;
    use bevy::input::keyboard::Key;
    use bevy::input::keyboard::KeyboardInput;
    use bevy::input::keyboard::NativeKey;
    use bevy::picking::InteractionPlugin;
    use bevy::picking::PickingPlugin;
    use bevy::picking::PickingSettings;
    use bevy::picking::PickingSystems;
    use bevy::picking::backend::HitData;
    use bevy::picking::backend::PointerHits;
    use bevy::picking::events::PointerState;
    use bevy::picking::events::pointer_events;
    use bevy::picking::hover::HoverMap;
    use bevy::picking::hover::PreviousHoverMap;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerAction;
    use bevy::picking::pointer::PointerId;
    use bevy::picking::pointer::PointerInput;
    use bevy::picking::pointer::PointerLocation;
    use bevy::picking::pointer::PointerMap;
    use bevy::prelude::App;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::Camera;
    use bevy::prelude::Color;
    use bevy::prelude::Entity;
    use bevy::prelude::EntityEvent;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::Pointer;
    use bevy::prelude::PointerButton;
    use bevy::prelude::PointerCancel;
    use bevy::prelude::PointerDrag;
    use bevy::prelude::PointerPress;
    use bevy::prelude::PointerRelease;
    use bevy::prelude::PreUpdate;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::prelude::Window;
    use bevy::prelude::default;
    use bevy::window::PrimaryWindow;
    use bevy::window::WindowRef;

    use super::super::*;
    use super::*;
    use crate::AlignX;
    use crate::AlignY;
    use crate::Anchor;
    use crate::Appearance;
    use crate::Border;
    use crate::ButtonClicked;
    use crate::ButtonPressed;
    use crate::ButtonReleased;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::RequestWidgetFocus;
    use crate::SliderReachableBand;
    use crate::WidgetInputPlugin;
    use crate::WidgetInteractivity;
    use crate::WidgetStepDirection;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::ButtonPress;
    use crate::widgets::SemanticWidgetIntent;
    use crate::widgets::VisualSlotId;
    use crate::widgets::WidgetKind;
    use crate::widgets::WidgetOf;
    use crate::widgets::WidgetVisualSlots;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::interaction::WidgetCaptures;

    const STATE_FOCUS_BORDER: Color = Color::srgb(0.9, 0.8, 0.2);
    const STATE_HOVER_FILL: Color = Color::srgb(0.2, 0.4, 0.6);
    const THUMB_ID: &str = "thumb";

    fn plain_slider(value: f32) -> Slider {
        Slider::new(0.0..=1.0)
            .default_value(value)
            .direction(SliderDirection::LeftToRight)
    }

    fn thumb_slider_tree(id: &str, slider: Slider, thumb_size: f32) -> LayoutTree {
        thumb_slider_tree_with(id, slider, thumb_size, |element| element)
    }

    fn thumb_slider_tree_with(
        id: &str,
        slider: Slider,
        thumb_size: f32,
        configure: impl FnOnce(SliderElement<Overlay>) -> SliderElement<Overlay>,
    ) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            configure(
                El::overlay()
                    .size(40.0, 16.0)
                    .background(Color::WHITE)
                    .border(Border::all(1.0, Color::WHITE))
                    .alignment(AlignX::Left, AlignY::Center)
                    .widget(id, slider),
            ),
            |builder| {
                builder.with(
                    El::new()
                        .size(thumb_size, 8.0)
                        .background(Color::BLACK)
                        .border(Border::all(1.0, Color::BLACK))
                        .id(THUMB_ID)
                        .slider_thumb(),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    fn slots_of(app: &App, widget: Entity) -> WidgetVisualSlots {
        app.world()
            .get::<WidgetVisualSlots>(widget)
            .cloned()
            .expect("widget should carry visual slots")
    }

    fn set_slider_value(app: &mut App, widget: Entity, value: f32) {
        let mut state = app
            .world_mut()
            .get_mut::<SliderState>(widget)
            .expect("widget should carry slider state");
        state.set_value(value).expect("value should apply");
    }

    /// Hover, press, and focus layers shared by the peer-isolation trees.
    fn apply_state_appearance<L>(element: SliderElement<L>) -> SliderElement<L> {
        element
            .hovered(Appearance::new().background(STATE_HOVER_FILL))
            .pressed(Appearance::new().background(STATE_PRESS_FILL))
            .focused(Appearance::new().border_color(STATE_FOCUS_BORDER))
    }

    /// A slider element in child layout `L`.
    type SliderElement<L> = El<L, crate::WidgetElement<Slider>>;
    type Overlay = crate::Overlay;
    const STATE_PRESS_FILL: Color = Color::srgb(0.6, 0.3, 0.1);

    #[derive(Default, Resource)]
    struct RecordedProposals(Vec<(Entity, f32, bool, SliderProposalSource)>);

    fn record_proposal(change: On<SliderChangeRequested>, mut recorded: ResMut<RecordedProposals>) {
        recorded.0.push((
            change.event_target(),
            change.value.get(),
            change.is_final,
            change.source,
        ));
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin))
            .init_resource::<RecordedProposals>()
            .add_observer(record_proposal);
        app
    }
    fn slider_tree(id: &str, slider: Slider) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().size(20.0, 10.0).widget(id, slider), |_| {});
        builder.build()
    }

    fn spawn_panel(app: &mut App, tree: LayoutTree) -> Entity {
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build()
            .expect("panel should build");
        app.world_mut().spawn(panel).id()
    }

    fn resolve_widget(app: &mut App, panel: Entity, id: &str) -> Entity {
        let id = PanelElementId::named(id);
        app.world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id))
            .ok()
            .flatten()
            .expect("widget should be reified")
    }

    fn slider_value(app: &App, widget: Entity) -> f32 {
        app.world()
            .get::<SliderState>(widget)
            .expect("widget should carry slider state")
            .value()
    }

    #[track_caller]
    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    const CAPTURE_VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);
    const CAPTURE_POINTER: PointerId = PointerId::Touch(11);

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Lifecycle {
        Grabbed,
        Proposal { is_final: bool },
        Released,
        Canceled(SliderCancelCause),
    }

    #[derive(Default, Resource)]
    struct RecordedLifecycle(Vec<Lifecycle>);

    fn record_grabbed(_: On<SliderGrabbed>, mut log: ResMut<RecordedLifecycle>) {
        log.0.push(Lifecycle::Grabbed);
    }

    fn record_lifecycle_proposal(
        change: On<SliderChangeRequested>,
        mut log: ResMut<RecordedLifecycle>,
    ) {
        log.0.push(Lifecycle::Proposal {
            is_final: change.is_final,
        });
    }

    fn record_released(_: On<SliderReleased>, mut log: ResMut<RecordedLifecycle>) {
        log.0.push(Lifecycle::Released);
    }

    fn record_canceled(event: On<SliderCanceled>, mut log: ResMut<RecordedLifecycle>) {
        log.0.push(Lifecycle::Canceled(event.cause));
    }

    struct Scene {
        app:    App,
        widget: Entity,
        panel:  Entity,
        camera: Entity,
        target: NormalizedRenderTarget,
    }

    fn seeded_camera() -> Camera {
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: CAPTURE_VIEWPORT.as_uvec2(),
            scale_factor:  1.0,
        });
        let mut projection = Projection::Perspective(PerspectiveProjection::default());
        projection.update(CAPTURE_VIEWPORT.x, CAPTURE_VIEWPORT.y);
        camera.computed.clip_from_view = projection.get_clip_from_view();
        camera
    }

    fn seeded_camera_transform() -> Transform {
        Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y)
    }

    fn capture_app() -> App {
        let mut app = test_app();
        app.init_resource::<RecordedLifecycle>()
            .add_observer(record_grabbed)
            .add_observer(record_lifecycle_proposal)
            .add_observer(record_released)
            .add_observer(record_canceled);
        app
    }

    fn projecting_panel(app: &mut App, tree: LayoutTree) -> Entity {
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .world_height(0.5)
            .with_tree(tree)
            .build()
            .expect("panel should build");
        app.world_mut().spawn(panel).id()
    }

    fn projecting_scene(tree: LayoutTree) -> Scene {
        let mut app = capture_app();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let camera = app
            .world_mut()
            .spawn((
                seeded_camera(),
                seeded_camera_transform(),
                GlobalTransform::from(seeded_camera_transform()),
                RenderTarget::Window(WindowRef::Primary),
            ))
            .id();
        let panel = projecting_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let target = RenderTarget::Window(WindowRef::Primary)
            .normalize(Some(window))
            .expect("primary window target normalizes");
        Scene {
            app,
            widget,
            panel,
            camera,
            target,
        }
    }

    fn ltr_scene() -> Scene {
        let slider = Slider::new(0.0..=1.0).value(0.5);
        projecting_scene(slider_tree("level", slider))
    }

    /// A projecting scene whose panel is center-anchored, so a press at the
    /// viewport center projects to the panel's layout center (authored x = 50).
    fn center_anchored_scene(tree: LayoutTree) -> Scene {
        let mut app = capture_app();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let camera = app
            .world_mut()
            .spawn((
                seeded_camera(),
                seeded_camera_transform(),
                GlobalTransform::from(seeded_camera_transform()),
                RenderTarget::Window(WindowRef::Primary),
            ))
            .id();
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .world_height(0.5)
            .anchor(Anchor::Center)
            .with_tree(tree)
            .build()
            .expect("panel should build");
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let target = RenderTarget::Window(WindowRef::Primary)
            .normalize(Some(window))
            .expect("primary window target normalizes");
        Scene {
            app,
            widget,
            panel,
            camera,
            target,
        }
    }

    fn press_at(scene: &mut Scene, camera: Entity, position: Vec2) {
        scene.app.world_mut().trigger(PointerPress {
            entity:  scene.widget,
            pointer: Pointer::new(
                CAPTURE_POINTER,
                Location {
                    target: scene.target.clone(),
                    position,
                },
            ),
            button:  PointerButton::Primary,
            hit:     HitData::new(camera, 0.0, None, None),
            count:   1,
        });
        scene.app.world_mut().flush();
    }

    fn press(scene: &mut Scene, position: Vec2) {
        let camera = scene.camera;
        press_at(scene, camera, position);
    }

    fn drag(scene: &mut Scene, position: Vec2) {
        scene.app.world_mut().trigger(PointerDrag {
            entity:   scene.widget,
            pointer:  Pointer::new(
                CAPTURE_POINTER,
                Location {
                    target: scene.target.clone(),
                    position,
                },
            ),
            button:   PointerButton::Primary,
            distance: Vec2::ZERO,
            delta:    Vec2::ZERO,
        });
        scene.app.world_mut().flush();
    }

    fn release(scene: &mut Scene, position: Vec2) {
        scene.app.world_mut().trigger(PointerRelease {
            entity:  scene.widget,
            pointer: Pointer::new(
                CAPTURE_POINTER,
                Location {
                    target: scene.target.clone(),
                    position,
                },
            ),
            button:  PointerButton::Primary,
            hit:     HitData::new(scene.camera, 0.0, None, None),
        });
        scene.app.world_mut().flush();
    }

    fn pointer_cancel(scene: &mut Scene) {
        scene.app.world_mut().trigger(PointerCancel {
            entity:  scene.widget,
            pointer: Pointer::new(
                CAPTURE_POINTER,
                Location {
                    target:   scene.target.clone(),
                    position: CAPTURE_VIEWPORT * 0.5,
                },
            ),
            hit:     HitData::new(scene.camera, 0.0, None, None),
        });
        scene.app.world_mut().flush();
    }

    fn lifecycle(scene: &Scene) -> Vec<Lifecycle> {
        scene.app.world().resource::<RecordedLifecycle>().0.clone()
    }

    fn is_dragging(scene: &Scene) -> bool {
        scene.app.world().get::<SliderDrag>(scene.widget).is_some()
    }

    fn captures_empty(scene: &Scene) -> bool {
        scene.app.world().resource::<SliderCaptures>().is_empty()
    }

    fn applied_value(scene: &Scene) -> f32 { slider_value(&scene.app, scene.widget) }

    fn latest_target(scene: &Scene) -> f32 {
        scene
            .app
            .world()
            .resource::<SliderCaptures>()
            .latest_raw_target(scene.widget)
            .expect("a live drag stores a raw target")
    }

    fn last_proposal_value(scene: &Scene) -> f32 {
        scene
            .app
            .world()
            .resource::<RecordedProposals>()
            .0
            .last()
            .expect("a proposal is recorded")
            .1
    }

    #[test]
    fn press_projects_grabs_and_proposes_once() {
        let mut scene = ltr_scene();
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);

        assert_eq!(
            lifecycle(&scene),
            [Lifecycle::Grabbed, Lifecycle::Proposal { is_final: false },]
        );
        assert!(is_dragging(&scene));
        assert!(!captures_empty(&scene));
        assert!(
            scene
                .app
                .world()
                .resource::<SliderCaptures>()
                .latest_raw_target(scene.widget)
                .is_some_and(f32::is_finite)
        );
    }

    #[test]
    fn press_without_projectable_camera_claims_nothing() {
        let mut scene = ltr_scene();
        let bogus_camera = scene.app.world_mut().spawn_empty().id();
        press_at(&mut scene, bogus_camera, CAPTURE_VIEWPORT * 0.5);

        assert_eq!(lifecycle(&scene), []);
        assert!(!is_dragging(&scene));
        assert!(captures_empty(&scene));
    }

    fn fine_slider_scene() -> Scene {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .size(100.0, 50.0)
                .widget("level", Slider::new(0.0..=1.0).value(0.5)),
            |_| {},
        );
        let mut scene = center_anchored_scene(builder.build());
        scene.app.add_observer(slider_self_update);
        scene.app.init_resource::<ButtonInput<KeyCode>>();
        scene
    }

    fn precision(scene: &mut Scene, held: bool) {
        let mut keys = scene.app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        if held {
            keys.press(KeyCode::ShiftLeft);
        } else {
            keys.release(KeyCode::ShiftLeft);
        }
        scene.app.update();
    }

    fn wide_fine_slider_scene(value: f32) -> Scene {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .size(100.0, 50.0)
                .widget("level", Slider::new(-1e9..=1e9).value(value)),
            |_| {},
        );
        let mut scene = center_anchored_scene(builder.build());
        scene.app.add_observer(slider_self_update);
        scene.app.init_resource::<ButtonInput<KeyCode>>();
        scene
    }

    #[test]
    fn shift_click_without_motion_preserves_a_small_value_in_a_wide_range() {
        let mut scene = wide_fine_slider_scene(20.0);
        precision(&mut scene, true);
        let origin = CAPTURE_VIEWPORT * 0.5;
        press(&mut scene, origin);
        drag(&mut scene, origin);
        release(&mut scene, origin);
        assert_eq!(applied_value(&scene).to_bits(), 20.0_f32.to_bits());
        assert!(
            scene
                .app
                .world()
                .resource::<RecordedProposals>()
                .0
                .iter()
                .all(|(_, value, _, _)| value.to_bits() == 20.0_f32.to_bits())
        );
    }

    #[test]
    fn fine_motion_keeps_small_accepted_offsets_in_a_wide_range() {
        let mut offset = wide_fine_slider_scene(20.0);
        let mut zero = wide_fine_slider_scene(0.0);
        let origin = CAPTURE_VIEWPORT * 0.5;
        let end = origin + Vec2::new(10.0, 0.0);
        for scene in [&mut offset, &mut zero] {
            precision(scene, true);
            press(scene, origin);
            drag(scene, end);
            release(scene, end);
        }
        assert!(
            (applied_value(&offset) - applied_value(&zero) - 20.0).abs() < 0.5,
            "relative motion must preserve the accepted offset, not round it to a track position"
        );
        assert!(applied_value(&zero) > 0.0);
    }

    #[test]
    fn fine_slider_travel_is_one_tenth_and_release_keeps_the_value() {
        let mut normal = fine_slider_scene();
        let mut fine = fine_slider_scene();
        let origin = CAPTURE_VIEWPORT * 0.5;
        precision(&mut fine, true);
        press(&mut normal, origin);
        press(&mut fine, origin);
        let start_normal = applied_value(&normal);
        let start_fine = applied_value(&fine);
        let end = origin + Vec2::new(8.0, 0.0);
        drag(&mut normal, end);
        drag(&mut fine, end);
        let ordinary_travel = applied_value(&normal) - start_normal;
        assert!(ordinary_travel > 0.01);
        assert_close(applied_value(&fine) - start_fine, ordinary_travel / 10.0);
        let held_value = applied_value(&fine);
        release(&mut fine, end);
        assert_close(applied_value(&fine), held_value);
    }

    #[test]
    fn slider_modifier_flips_reanchor_without_jumping() {
        let mut scene = fine_slider_scene();
        let origin = CAPTURE_VIEWPORT * 0.5;
        press(&mut scene, origin);
        let first = origin + Vec2::new(4.0, 0.0);
        drag(&mut scene, first);
        let ordinary = applied_value(&scene);
        precision(&mut scene, true);
        drag(&mut scene, first);
        assert_close(applied_value(&scene), ordinary);
        let second = first + Vec2::new(4.0, 0.0);
        drag(&mut scene, second);
        let fine = applied_value(&scene);
        assert!(fine > ordinary);
        precision(&mut scene, false);
        drag(&mut scene, second);
        assert_close(applied_value(&scene), fine);
        drag(&mut scene, second + Vec2::new(4.0, 0.0));
        assert_close(applied_value(&scene) - fine, (fine - ordinary) * 10.0);
    }

    #[test]
    fn fine_slider_motion_continues_beyond_the_track_ends() {
        let mut ordinary = fine_slider_scene();
        let mut fine = fine_slider_scene();
        let origin = CAPTURE_VIEWPORT * 0.5;
        press(&mut ordinary, origin);
        let start = applied_value(&ordinary);
        drag(&mut ordinary, origin + Vec2::new(8.0, 0.0));
        let ordinary_travel = applied_value(&ordinary) - start;
        precision(&mut fine, true);
        press(&mut fine, origin);
        drag(&mut fine, origin + Vec2::new(120.0, 0.0));
        assert_close(applied_value(&fine), ordinary_travel.mul_add(1.5, start));
    }

    #[test]
    fn click_without_drag_proposes_release_value_before_released() {
        let mut scene = ltr_scene();
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        release(&mut scene, CAPTURE_VIEWPORT * 0.5);

        assert_eq!(
            lifecycle(&scene),
            [
                Lifecycle::Grabbed,
                Lifecycle::Proposal { is_final: false },
                Lifecycle::Proposal { is_final: true },
                Lifecycle::Released,
            ]
        );
        assert!(!is_dragging(&scene));
        assert!(captures_empty(&scene));
    }

    #[test]
    fn each_drag_reprojects_and_re_stores_the_raw_target() {
        let mut scene = ltr_scene();
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        drag(
            &mut scene,
            CAPTURE_VIEWPORT.mul_add(Vec2::splat(0.5), Vec2::new(400.0, 0.0)),
        );
        let after_right = latest_target(&scene);
        drag(
            &mut scene,
            CAPTURE_VIEWPORT.mul_add(Vec2::splat(0.5), Vec2::new(-400.0, 0.0)),
        );
        let after_left = latest_target(&scene);

        // The grab plus two drags each emit exactly one non-final proposal.
        assert_eq!(
            lifecycle(&scene),
            [
                Lifecycle::Grabbed,
                Lifecycle::Proposal { is_final: false },
                Lifecycle::Proposal { is_final: false },
                Lifecycle::Proposal { is_final: false },
            ]
        );
        // Two opposite drags replace the stored raw target with different
        // values: dragging right past the travel reprojects to the range end,
        // dragging left reprojects to the range start.
        assert_close(after_right, 1.0);
        assert_close(after_left, 0.0);
        assert!(after_right > after_left);
        // The last stored raw target matches the last proposal that drag
        // emitted.
        assert_close(after_left, last_proposal_value(&scene));
    }

    #[test]
    fn projection_loss_after_capture_cancels_exactly_once() {
        let mut scene = ltr_scene();
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        scene.app.world_mut().entity_mut(scene.camera).despawn();
        drag(&mut scene, CAPTURE_VIEWPORT * 0.5);

        assert_eq!(
            lifecycle(&scene),
            [
                Lifecycle::Grabbed,
                Lifecycle::Proposal { is_final: false },
                Lifecycle::Canceled(SliderCancelCause::ProjectionUnavailable),
            ]
        );
        assert!(!is_dragging(&scene));
        assert!(captures_empty(&scene));

        // A later drag on the freed widget proposes and cancels nothing.
        drag(&mut scene, CAPTURE_VIEWPORT * 0.5);
        assert_eq!(lifecycle(&scene).len(), 3);
    }

    #[test]
    fn release_away_still_proposes_the_release_value() {
        let mut scene = ltr_scene();
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        release(
            &mut scene,
            CAPTURE_VIEWPORT.mul_add(Vec2::splat(0.5), Vec2::new(400.0, 0.0)),
        );

        assert_eq!(
            lifecycle(&scene),
            [
                Lifecycle::Grabbed,
                Lifecycle::Proposal { is_final: false },
                Lifecycle::Proposal { is_final: true },
                Lifecycle::Released,
            ]
        );
        assert!(captures_empty(&scene));
    }

    #[test]
    fn rejecting_every_proposal_leaves_applied_state_unchanged() {
        let mut scene = ltr_scene();
        let before = applied_value(&scene);
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        // Release past the right edge of the travel so the final proposal
        // carries the clamped raw release-position value: the range end.
        release(
            &mut scene,
            CAPTURE_VIEWPORT.mul_add(Vec2::splat(0.5), Vec2::new(400.0, 0.0)),
        );

        assert!(matches!(
            lifecycle(&scene).last(),
            Some(Lifecycle::Released)
        ));
        // No proposal was accepted, so the applied value never left its
        // authored default.
        assert_close(applied_value(&scene), before);
        // The final proposal still carries the raw release-position value,
        // independent of the untouched applied state.
        let proposals = &scene.app.world().resource::<RecordedProposals>().0;
        let (_, final_value, is_final, _) =
            *proposals.last().expect("a final proposal is recorded");
        assert!(is_final, "the last proposal completes the interaction");
        assert_close(final_value, 1.0);
    }

    #[test]
    fn opt_in_self_update_applies_each_proposal_once() {
        let mut scene = ltr_scene();
        scene.app.add_observer(slider_self_update);

        // Each proposal the interaction emits is applied exactly once, so the
        // applied value tracks every proposal in turn rather than only the
        // final one.
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        assert_close(applied_value(&scene), last_proposal_value(&scene));
        drag(
            &mut scene,
            CAPTURE_VIEWPORT.mul_add(Vec2::splat(0.5), Vec2::new(400.0, 0.0)),
        );
        assert_close(applied_value(&scene), 1.0);
        assert_close(applied_value(&scene), last_proposal_value(&scene));
        drag(
            &mut scene,
            CAPTURE_VIEWPORT.mul_add(Vec2::splat(0.5), Vec2::new(-400.0, 0.0)),
        );
        assert_close(applied_value(&scene), 0.0);
        assert_close(applied_value(&scene), last_proposal_value(&scene));
        release(&mut scene, CAPTURE_VIEWPORT * 0.5);
        assert_close(applied_value(&scene), last_proposal_value(&scene));

        // The grab, two drags, and the final release each contributed exactly
        // one accepted proposal.
        let proposals = &scene.app.world().resource::<RecordedProposals>().0;
        assert_eq!(proposals.len(), 4);
        assert!(
            proposals[3].2,
            "the last proposal completes the interaction"
        );
    }

    #[test]
    fn disable_cancels_an_active_drag() {
        let mut scene = ltr_scene();
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        let panel = scene.panel;
        // Re-author the same slider as disabled: the same-kind refresh keeps
        // the capture, and the derived `WidgetDisabled` marker cancels it.
        let slider = Slider::new(0.0..=1.0).value(0.5);
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .size(20.0, 10.0)
                .widget("level", slider)
                .widget_interactivity(WidgetInteractivity::Disabled),
            |_| {},
        );
        scene
            .app
            .world_mut()
            .commands()
            .set_tree(panel, builder.build())
            .expect("disabled tree should be accepted");
        scene.app.update();

        assert_eq!(
            lifecycle(&scene).last(),
            Some(&Lifecycle::Canceled(SliderCancelCause::Disabled))
        );
        assert!(!is_dragging(&scene));
        assert!(captures_empty(&scene));
    }

    #[test]
    fn pointer_cancel_terminates_the_drag() {
        let mut scene = ltr_scene();
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        pointer_cancel(&mut scene);

        assert_eq!(
            lifecycle(&scene).last(),
            Some(&Lifecycle::Canceled(SliderCancelCause::PointerCanceled))
        );
        assert!(captures_empty(&scene));
    }

    #[test]
    fn semantic_step_proposes_once_and_active_capture_blocks_later_steps() {
        let mut scene = ltr_scene();
        let widget = scene.widget;
        scene.app.world_mut().trigger(SemanticWidgetIntent::Step {
            entity:    widget,
            direction: WidgetStepDirection::Right,
        });
        scene.app.world_mut().flush();
        let proposals = &scene.app.world().resource::<RecordedProposals>().0;
        assert_eq!(proposals.len(), 1);
        assert!((proposals[0].1 - 0.55).abs() < 1e-6);
        assert_eq!(proposals[0].3, SliderProposalSource::Semantic);

        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        let count = scene.app.world().resource::<RecordedProposals>().0.len();
        scene.app.world_mut().trigger(SemanticWidgetIntent::Step {
            entity:    widget,
            direction: WidgetStepDirection::Right,
        });
        scene.app.world_mut().flush();
        assert_eq!(
            scene.app.world().resource::<RecordedProposals>().0.len(),
            count
        );
    }

    #[test]
    fn pointer_capture_record_blocks_an_immediate_semantic_step() {
        let mut scene = ltr_scene();
        scene.app.world_mut().trigger(PointerPress {
            entity:  scene.widget,
            pointer: Pointer::new(
                CAPTURE_POINTER,
                Location {
                    target:   scene.target.clone(),
                    position: CAPTURE_VIEWPORT * 0.5,
                },
            ),
            button:  PointerButton::Primary,
            hit:     HitData::new(scene.camera, 0.0, None, None),
            count:   1,
        });
        assert!(
            scene
                .app
                .world()
                .resource::<SliderCaptures>()
                .has_drag_or_pending_press(scene.widget)
        );
        assert!(scene.app.world().get::<SliderDrag>(scene.widget).is_none());
        let semantic_count = scene
            .app
            .world()
            .resource::<RecordedProposals>()
            .0
            .iter()
            .filter(|proposal| proposal.3 == SliderProposalSource::Semantic)
            .count();
        scene.app.world_mut().trigger(SemanticWidgetIntent::Step {
            entity:    scene.widget,
            direction: WidgetStepDirection::Right,
        });
        scene.app.world_mut().flush();
        let proposals = &scene.app.world().resource::<RecordedProposals>().0;
        assert!(
            proposals
                .iter()
                .any(|proposal| { proposal.3 == SliderProposalSource::Pointer(CAPTURE_POINTER) })
        );
        assert_eq!(
            proposals
                .iter()
                .filter(|proposal| proposal.3 == SliderProposalSource::Semantic)
                .count(),
            semantic_count
        );
    }

    #[test]
    fn semantic_cancel_terminates_and_is_a_noop_without_capture() {
        let mut scene = ltr_scene();
        let widget = scene.widget;
        scene
            .app
            .world_mut()
            .trigger(SemanticWidgetIntent::Cancel { entity: widget });
        scene.app.world_mut().flush();
        assert_eq!(lifecycle(&scene), []);

        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        scene
            .app
            .world_mut()
            .trigger(SemanticWidgetIntent::Cancel { entity: widget });
        scene.app.world_mut().flush();

        assert_eq!(
            lifecycle(&scene).last(),
            Some(&Lifecycle::Canceled(SliderCancelCause::Explicit))
        );
        assert!(captures_empty(&scene));
    }

    #[test]
    fn tree_removal_cancels_the_active_drag() {
        let mut scene = ltr_scene();
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        let panel = scene.panel;
        scene
            .app
            .world_mut()
            .commands()
            .set_tree(panel, LayoutBuilder::new(100.0, 50.0).build())
            .expect("empty tree should be accepted");
        scene.app.update();

        assert_eq!(
            lifecycle(&scene).last(),
            Some(&Lifecycle::Canceled(SliderCancelCause::WidgetRemoved))
        );
        assert!(captures_empty(&scene));
    }

    #[test]
    fn kind_change_cancels_the_active_drag() {
        let mut scene = ltr_scene();
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        let panel = scene.panel;
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().size(20.0, 10.0).button("level"), |_| {});
        scene
            .app
            .world_mut()
            .commands()
            .set_tree(panel, builder.build())
            .expect("button tree should be accepted");
        scene.app.update();

        assert_eq!(
            lifecycle(&scene).last(),
            Some(&Lifecycle::Canceled(SliderCancelCause::WidgetKindChanged))
        );
        assert_eq!(
            scene.app.world().get::<WidgetKind>(scene.widget),
            Some(&WidgetKind::Button)
        );
        assert!(captures_empty(&scene));
    }

    #[test]
    fn panel_role_removal_finalizes_the_active_drag() {
        let mut scene = ltr_scene();
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        let panel = scene.panel;
        scene
            .app
            .world_mut()
            .entity_mut(panel)
            .remove::<DiegeticPanel>();
        scene.app.world_mut().flush();

        assert_eq!(
            lifecycle(&scene).last(),
            Some(&Lifecycle::Canceled(SliderCancelCause::WidgetRemoved))
        );
        assert!(captures_empty(&scene));
    }

    #[test]
    fn full_panel_despawn_finalizes_the_active_drag() {
        let mut scene = ltr_scene();
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        let panel = scene.panel;
        scene.app.world_mut().entity_mut(panel).despawn();
        scene.app.world_mut().flush();

        assert_eq!(
            lifecycle(&scene).last(),
            Some(&Lifecycle::Canceled(SliderCancelCause::WidgetRemoved))
        );
        assert!(captures_empty(&scene));
    }

    #[test]
    fn slider_root_content_box_excludes_border_and_padding_through_reify() {
        let slider = Slider::new(0.0..=1.0).value(0.5);
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .size(40.0, 20.0)
                .border(crate::Border::all(2.0, Color::WHITE))
                .padding(crate::Padding::all(3.0))
                .widget("level", slider),
            |_| {},
        );
        let mut app = capture_app();
        let panel = projecting_panel(&mut app, builder.build());
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");

        let slots = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .expect("reified slider carries its visual slots");
        let content = slots
            .content_box(VisualSlotId::SLIDER_ROOT)
            .expect("the slider root slot records a content box");
        let border = slots
            .border_box(VisualSlotId::SLIDER_ROOT)
            .expect("the slider root slot records a border box");
        // The 2-point border and 3-point padding inset every content edge, so
        // the content box sits strictly inside the border box on both axes.
        // The panel scales the authored tree, so the inset is compared in the
        // solved space rather than against the authored 40x20 size.
        assert!(content.width > 0.0 && content.height > 0.0);
        assert!(content.width < border.width);
        assert!(content.height < border.height);
        assert!(content.x > border.x);
        assert!(content.y > border.y);
        // Border (2) plus padding (3) inset each edge equally, so the left and
        // right insets match and the content box stays centered. The scaled
        // layout carries float rounding, so compare with a small tolerance.
        let left_inset = content.x - border.x;
        let right_inset = (border.x + border.width) - (content.x + content.width);
        assert!(left_inset > 0.0);
        assert!((left_inset - right_inset).abs() < 1e-3);
    }

    #[test]
    fn projected_interior_value_reads_authored_border_and_padding() {
        // Authored panel-wide slider root with a 2-point border and 3-point
        // padding, mapped to a range whose interior midpoint (2.0) is an
        // independently known value that no clamp to 0.0 or 1.0 can produce.
        // The root spans the 100-point authored width so the panel's layout
        // center (authored x = 50) falls inside its content box.
        let slider = Slider::new(0.0..=4.0).value(2.0);
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .size(100.0, 20.0)
                .border(crate::Border::all(2.0, Color::WHITE))
                .padding(crate::Padding::all(3.0))
                .widget("level", slider),
            |_| {},
        );
        let mut scene = center_anchored_scene(builder.build());

        let (content, border) = {
            let slots = scene
                .app
                .world()
                .get::<WidgetVisualSlots>(scene.widget)
                .expect("reified slider carries slots");
            (
                slots
                    .content_box(VisualSlotId::SLIDER_ROOT)
                    .expect("content box"),
                slots
                    .border_box(VisualSlotId::SLIDER_ROOT)
                    .expect("border box"),
            )
        };

        // The solved layout scales the authored tree uniformly; the border box
        // (authored width 100) fixes points-per-authored-unit independently of
        // the content-box computation under test.
        let scale = border.width / 100.0;
        // The content box excludes exactly the authored 2-point border and
        // 3-point padding on every side: width 100 - 2*2 - 2*3 = 90, left inset
        // 2 + 3 = 5.
        assert_close(content.width, 90.0 * scale);
        assert_close(content.x, 5.0f32.mul_add(scale, border.x));

        // A press at the viewport center projects to the panel's layout center
        // (authored x = 50). Through the live camera → panel-ray → content-box
        // chain, that interior travel position must map to the range value the
        // reified content box places there — not a clamp to an endpoint.
        let panel_center = 50.0 * scale;
        let fraction = (panel_center - content.x) / content.width;
        let expected = 4.0 * fraction;
        assert!(
            expected > 0.1 && expected < 3.9,
            "expected an interior value, got {expected}"
        );
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        assert_close(last_proposal_value(&scene), expected);
    }

    // ---- real Bevy dispatcher integration ----
    //
    // These fixtures drive the shipped hover and `pointer_events` systems with
    // synthetic `PointerHits` and raw `PointerInput`, then reconcile through the
    // shared `reconcile_pointer_input` dispatcher. They never move or simulate
    // an operating-system pointer; every hit and action is authored directly.

    const DISPATCH_POINTER: PointerId = PointerId::Touch(70);
    const SECOND_POINTER: PointerId = PointerId::Touch(71);

    fn center() -> Vec2 { CAPTURE_VIEWPORT * 0.5 }

    fn past_right() -> Vec2 { CAPTURE_VIEWPORT.mul_add(Vec2::splat(0.5), Vec2::new(400.0, 0.0)) }

    /// A projecting panel wired to the shipped hover and pointer-event systems
    /// plus the shared raw dispatcher.
    struct DispatchScene {
        app:    App,
        panel:  Entity,
        camera: Entity,
        target: NormalizedRenderTarget,
    }

    #[derive(Default, Resource)]
    struct ButtonLog(Vec<&'static str>);

    fn log_button_pressed(_: On<ButtonPressed>, mut log: ResMut<ButtonLog>) {
        log.0.push("pressed");
    }

    fn log_button_released(_: On<ButtonReleased>, mut log: ResMut<ButtonLog>) {
        log.0.push("released");
    }

    fn log_button_clicked(_: On<ButtonClicked>, mut log: ResMut<ButtonLog>) {
        log.0.push("clicked");
    }

    fn dispatch_scene(tree: LayoutTree) -> DispatchScene {
        let mut app = capture_app();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let camera = app
            .world_mut()
            .spawn((
                seeded_camera(),
                seeded_camera_transform(),
                GlobalTransform::from(seeded_camera_transform()),
                RenderTarget::Window(WindowRef::Primary),
            ))
            .id();
        app.add_plugins(InteractionPlugin)
            .add_message::<PointerInput>()
            .add_message::<PointerHits>()
            .init_resource::<PointerMap>()
            .configure_sets(
                PreUpdate,
                (PickingSystems::Hover, PickingSystems::Last).chain(),
            );
        let panel = projecting_panel(&mut app, tree);
        app.update();
        let target = RenderTarget::Window(WindowRef::Primary)
            .normalize(Some(window))
            .expect("primary window target normalizes");
        DispatchScene {
            app,
            panel,
            camera,
            target,
        }
    }

    fn record_buttons(scene: &mut DispatchScene) {
        scene
            .app
            .init_resource::<ButtonLog>()
            .add_observer(log_button_pressed)
            .add_observer(log_button_released)
            .add_observer(log_button_clicked);
    }

    fn add_pointer(scene: &mut DispatchScene, pointer_id: PointerId) -> Entity {
        let location = Location {
            target:   scene.target.clone(),
            position: center(),
        };

        scene
            .app
            .world_mut()
            .spawn((pointer_id, PointerLocation::new(location)))
            .id()
    }

    /// Feeds one synthetic backend hit so the shipped hover systems place
    /// `widget` under `pointer_id` with `camera` as the hit camera.
    fn feed_hit(scene: &mut DispatchScene, pointer_id: PointerId, camera: Entity, widget: Entity) {
        scene.app.world_mut().write_message(PointerHits::new(
            pointer_id,
            vec![(widget, HitData::new(camera, 0.0, None, None))],
            0.0,
        ));
    }

    fn feed_input(
        scene: &mut DispatchScene,
        pointer_id: PointerId,
        position: Vec2,
        action: PointerAction,
    ) {
        let location = Location {
            target: scene.target.clone(),
            position,
        };
        scene
            .app
            .world_mut()
            .write_message(PointerInput::new(pointer_id, location, action));
    }

    /// Drives one complete click through Bevy's real pointer dispatcher.
    fn dispatch_click(scene: &mut DispatchScene, widget: Entity) {
        dispatch_click_with_jitter(scene, widget, Vec2::ZERO);
    }

    /// Adds a small dispatched drag between the real press and release.
    fn dispatch_click_with_jitter(scene: &mut DispatchScene, widget: Entity, jitter: Vec2) {
        crate::test_support::configure_click_interval(&mut scene.app, 2);
        let camera = scene.camera;
        feed_hit(scene, DISPATCH_POINTER, camera, widget);
        feed_input(
            scene,
            DISPATCH_POINTER,
            center(),
            PointerAction::Press(PointerButton::Primary),
        );
        scene.app.update();
        if jitter != Vec2::ZERO {
            scene.app.world_mut().trigger(PointerDrag {
                entity:   widget,
                pointer:  Pointer::new(
                    DISPATCH_POINTER,
                    Location {
                        target:   scene.target.clone(),
                        position: center() + jitter,
                    },
                ),
                button:   PointerButton::Primary,
                distance: jitter,
                delta:    jitter,
            });
            scene.app.world_mut().flush();
        }
        feed_hit(scene, DISPATCH_POINTER, camera, widget);
        feed_input(
            scene,
            DISPATCH_POINTER,
            center(),
            PointerAction::Release(PointerButton::Primary),
        );
        scene.app.update();
    }

    /// Sets the current and previous hover maps directly so raw-batch ordering
    /// is exact.
    fn set_hover(
        scene: &mut DispatchScene,
        pointer_id: PointerId,
        previous: &[Entity],
        current: &[Entity],
        camera: Entity,
    ) {
        let entries = |entities: &[Entity]| {
            entities
                .iter()
                .copied()
                .map(|entity| (entity, HitData::new(camera, 0.0, None, None)))
                .collect()
        };
        scene
            .app
            .world_mut()
            .resource_mut::<PreviousHoverMap>()
            .insert(pointer_id, entries(previous));
        scene
            .app
            .world_mut()
            .resource_mut::<HoverMap>()
            .insert(pointer_id, entries(current));
    }

    /// Drives one raw batch through the shipped `pointer_events` system and the
    /// shared reconciler, with the hover maps left exactly as `set_hover` fixed
    /// them.
    fn run_raw(scene: &mut DispatchScene, inputs: &[(PointerId, Vec2, PointerAction)]) {
        for &(pointer_id, position, action) in inputs {
            feed_input(scene, pointer_id, position, action);
        }
        let result = scene.app.world_mut().run_system_cached(pointer_events);
        assert!(result.is_ok());
        let result = scene
            .app
            .world_mut()
            .run_system_cached(crate::widgets::interaction::reconcile_pointer_input);
        assert!(result.is_ok());
    }

    fn dispatch_lifecycle(scene: &DispatchScene) -> Vec<Lifecycle> {
        scene.app.world().resource::<RecordedLifecycle>().0.clone()
    }

    fn owner_of(scene: &DispatchScene, widget: Entity) -> Option<PointerId> {
        scene
            .app
            .world()
            .resource::<WidgetCaptures>()
            .pointer(widget)
    }

    fn dragging(scene: &DispatchScene, widget: Entity) -> bool {
        scene.app.world().get::<SliderDrag>(widget).is_some()
    }

    fn dispatch_empty(scene: &DispatchScene) -> bool {
        scene.app.world().resource::<SliderCaptures>().is_empty()
            && scene.app.world().resource::<WidgetCaptures>().is_empty()
    }

    #[test]
    fn raw_middle_press_ends_slider_motion_without_changing_its_value() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        scene.app.add_observer(slider_self_update);
        add_pointer(&mut scene, DISPATCH_POINTER);
        let camera = scene.camera;
        feed_hit(&mut scene, DISPATCH_POINTER, camera, widget);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            center(),
            PointerAction::Press(PointerButton::Primary),
        );
        scene.app.update();
        assert!(dragging(&scene, widget));
        let end = center() + Vec2::new(4.0, 0.0);
        feed_hit(&mut scene, DISPATCH_POINTER, camera, widget);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            end,
            PointerAction::Move {
                delta: Vec2::new(4.0, 0.0),
            },
        );
        scene.app.update();
        let before = slider_value(&scene.app, widget);
        feed_hit(&mut scene, DISPATCH_POINTER, camera, widget);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            end,
            PointerAction::Press(PointerButton::Middle),
        );
        scene.app.update();
        assert!(!dragging(&scene, widget));
        assert!(dispatch_empty(&scene));
        for action in [
            PointerAction::Move {
                delta: Vec2::splat(10.0),
            },
            PointerAction::Release(PointerButton::Primary),
        ] {
            feed_hit(&mut scene, DISPATCH_POINTER, camera, widget);
            feed_input(
                &mut scene,
                DISPATCH_POINTER,
                end + Vec2::splat(10.0),
                action,
            );
            scene.app.update();
        }
        assert_close(slider_value(&scene.app, widget), before);
        assert_eq!(
            dispatch_lifecycle(&scene)
                .iter()
                .filter(|event| matches!(event, Lifecycle::Canceled(_)))
                .count(),
            1
        );
    }

    fn slider0() -> Slider { Slider::new(0.0..=1.0).value(0.5) }

    fn reset_slider_tree(slider: Slider) -> LayoutTree {
        let mut builder = LayoutBuilder::with_widget_root(
            El::overlay()
                .size(100.0, 50.0)
                .alignment(AlignX::Left, AlignY::Top)
                .widget("level", slider),
        );
        builder.with(El::new().size(10.0, 10.0).slider_thumb(), |_| {});
        builder.build()
    }

    fn button_and_slider_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .size(20.0, 10.0)
                .background(Color::WHITE)
                .button("act")
                .hovered(Appearance::new().background(STATE_HOVER_FILL))
                .pressed(Appearance::new().background(STATE_PRESS_FILL)),
            |_| {},
        );
        // The slider's state layers override background and border color, so the
        // element authors both a normal background and a normal border for the
        // surface those overrides patch.
        builder.with(
            apply_state_appearance(
                El::new()
                    .size(20.0, 10.0)
                    .background(Color::WHITE)
                    .border(Border::all(1.0, Color::WHITE))
                    .widget("level", slider0()),
            ),
            |_| {},
        );
        builder.with(
            apply_state_appearance(
                El::new()
                    .size(20.0, 10.0)
                    .background(Color::WHITE)
                    .border(Border::all(1.0, Color::WHITE))
                    .widget("peer", slider0()),
            ),
            |_| {},
        );
        builder.build()
    }
    fn two_slider_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new().size(20.0, 10.0).widget("first", slider0()),
            |_| {},
        );
        builder.with(
            El::new().size(20.0, 10.0).widget("second", slider0()),
            |_| {},
        );
        builder.build()
    }

    #[test]
    fn dispatched_press_grabs_and_proposes_once() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);

        feed_hit(&mut scene, DISPATCH_POINTER, camera, widget);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            center(),
            PointerAction::Press(PointerButton::Primary),
        );
        scene.app.update();

        assert_eq!(
            dispatch_lifecycle(&scene),
            [Lifecycle::Grabbed, Lifecycle::Proposal { is_final: false }]
        );
        assert!(dragging(&scene, widget));
        assert_eq!(owner_of(&scene, widget), Some(DISPATCH_POINTER));
        assert!(
            scene
                .app
                .world()
                .resource::<SliderCaptures>()
                .latest_raw_target(widget)
                .is_some_and(f32::is_finite)
        );
    }

    #[test]
    fn dispatched_press_without_projection_claims_nothing() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        add_pointer(&mut scene, DISPATCH_POINTER);
        let bogus_camera = scene.app.world_mut().spawn_empty().id();

        // The hit names a camera that cannot project, so the press claims and
        // emits nothing even though the dispatcher observed it.
        feed_hit(&mut scene, DISPATCH_POINTER, bogus_camera, widget);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            center(),
            PointerAction::Press(PointerButton::Primary),
        );
        scene.app.update();

        assert_eq!(dispatch_lifecycle(&scene), []);
        assert!(!dragging(&scene, widget));
        assert!(dispatch_empty(&scene));
    }

    #[test]
    fn dispatched_click_terminalizes_release_exactly_once() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);

        feed_hit(&mut scene, DISPATCH_POINTER, camera, widget);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            center(),
            PointerAction::Press(PointerButton::Primary),
        );
        scene.app.update();
        feed_hit(&mut scene, DISPATCH_POINTER, camera, widget);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            center(),
            PointerAction::Release(PointerButton::Primary),
        );
        scene.app.update();

        // The final release-position proposal precedes `SliderReleased`, and the
        // capture frees exactly once.
        assert_eq!(
            dispatch_lifecycle(&scene),
            [
                Lifecycle::Grabbed,
                Lifecycle::Proposal { is_final: false },
                Lifecycle::Proposal { is_final: true },
                Lifecycle::Released,
            ]
        );
        assert!(!dragging(&scene, widget));
        assert!(dispatch_empty(&scene));

        // A further quiet update emits nothing more.
        scene.app.update();
        assert_eq!(dispatch_lifecycle(&scene).len(), 4);
    }

    #[test]
    fn double_click_restores_an_off_step_authored_default_exactly() {
        let slider = Slider::new(0.0..=1.0).step(0.1).default_value(0.42);
        let mut scene = dispatch_scene(reset_slider_tree(slider));
        scene.app.add_observer(slider_self_update);
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        add_pointer(&mut scene, DISPATCH_POINTER);
        set_slider_value(&mut scene.app, widget, 0.0);
        scene.app.update();
        dispatch_click(&mut scene, widget);
        dispatch_click(&mut scene, widget);
        assert_eq!(
            slider_value(&scene.app, widget).to_bits(),
            0.42_f32.to_bits()
        );
        assert!(dispatch_empty(&scene));
    }

    #[test]
    fn a_thumb_press_without_motion_preserves_an_off_step_code_value() {
        let slider = Slider::new(0.0..=1.0).step(0.1);
        let mut scene = dispatch_scene(reset_slider_tree(slider));
        scene.app.add_observer(slider_self_update);
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        add_pointer(&mut scene, DISPATCH_POINTER);
        set_slider_value(&mut scene.app, widget, 0.02);
        scene.app.update();
        let camera = scene.camera;
        let world = scene.app.world();
        let state = world.get::<SliderState>(widget).expect("slider state");
        let slots = slots_of(&scene.app, widget);
        let thumb = slots
            .border_box(VisualSlotId::SLIDER_THUMB)
            .expect("drawn thumb");
        let offset = projection::thumb_translation(&slots, state).expect("thumb position");
        let (x, y) = thumb.center();
        let panel = world.get::<DiegeticPanel>(scene.panel).expect("panel");
        let (anchor_x, anchor_y) = panel.anchor_offsets();
        let scale = panel.points_to_world();
        let point = world
            .get::<GlobalTransform>(scene.panel)
            .expect("panel pose")
            .transform_point(Vec3::new(
                (x + offset.x).mul_add(scale, -anchor_x),
                (-(y + offset.y)).mul_add(scale, anchor_y),
                0.0,
            ));
        let position = world
            .get::<Camera>(camera)
            .expect("camera")
            .world_to_viewport(
                world.get::<GlobalTransform>(camera).expect("camera pose"),
                point,
            )
            .expect("thumb inside the view");
        feed_hit(&mut scene, DISPATCH_POINTER, camera, widget);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            position,
            PointerAction::Press(PointerButton::Primary),
        );
        scene.app.update();
        assert_eq!(
            slider_value(&scene.app, widget).to_bits(),
            0.02_f32.to_bits()
        );
        feed_hit(&mut scene, DISPATCH_POINTER, camera, widget);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            position,
            PointerAction::Release(PointerButton::Primary),
        );
        scene.app.update();
        assert_eq!(
            slider_value(&scene.app, widget).to_bits(),
            0.02_f32.to_bits()
        );
        assert!(dispatch_empty(&scene));
    }

    #[test]
    fn dispatched_double_click_resets_to_the_authored_default() {
        let slider = plain_slider(0.25);
        let mut scene = dispatch_scene(reset_slider_tree(slider));
        scene.app.add_observer(slider_self_update);
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        add_pointer(&mut scene, DISPATCH_POINTER);
        set_slider_value(&mut scene.app, widget, 0.0);
        scene.app.update();

        dispatch_click_with_jitter(&mut scene, widget, Vec2::new(2.0, 1.0));
        assert_close(slider_value(&scene.app, widget), 0.0);
        dispatch_click_with_jitter(&mut scene, widget, Vec2::new(1.0, 2.0));

        assert_close(slider_value(&scene.app, widget), 0.25);
        assert!(!dragging(&scene, widget));
        assert!(dispatch_empty(&scene));
        let proposals = &scene.app.world().resource::<RecordedProposals>().0;
        let (_, value, is_final, source) =
            *proposals.last().expect("reset proposal should be recorded");
        assert_close(value, 0.25);
        assert!(is_final);
        assert_eq!(source, SliderProposalSource::Pointer(DISPATCH_POINTER));
    }

    #[test]
    fn logarithmic_slider_double_click_after_rebuild_restores_exact_real_default() {
        let slider = Slider::new(0.01..=10.0)
            .logarithmic()
            .default_value(0.42)
            .value(0.173);
        let mut scene = dispatch_scene(reset_slider_tree(slider.clone()));
        scene.app.add_observer(slider_self_update);
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        add_pointer(&mut scene, DISPATCH_POINTER);
        assert_eq!(
            slider_value(&scene.app, widget).to_bits(),
            0.173_f32.to_bits()
        );
        // This dispatch harness clicks the start of the track; place the thumb there.
        set_slider_value(&mut scene.app, widget, 0.01);
        scene.app.update();
        scene
            .app
            .world_mut()
            .commands()
            .set_tree(scene.panel, reset_slider_tree(slider.value(3.2)))
            .expect("rebuilt log slider");
        scene.app.update();
        dispatch_click_with_jitter(&mut scene, widget, Vec2::new(2.0, 1.0));
        dispatch_click_with_jitter(&mut scene, widget, Vec2::new(1.0, 2.0));
        assert_eq!(
            slider_value(&scene.app, widget).to_bits(),
            0.42_f32.to_bits(),
            "reset accepts the authored real default without rounding"
        );
        let (_, value, is_final, source) = *scene
            .app
            .world()
            .resource::<RecordedProposals>()
            .0
            .last()
            .expect("real reset proposal");
        assert_eq!(value.to_bits(), 0.42_f32.to_bits());
        assert!(is_final);
        assert_eq!(source, SliderProposalSource::Pointer(DISPATCH_POINTER));
        assert!(!dragging(&scene, widget));
        assert!(dispatch_empty(&scene));
    }

    #[test]
    fn dispatched_double_click_after_rebuild_keeps_the_reset_default() {
        let slider = plain_slider(0.25);
        let mut scene = dispatch_scene(reset_slider_tree(slider));
        scene.app.add_observer(slider_self_update);
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        add_pointer(&mut scene, DISPATCH_POINTER);
        set_slider_value(&mut scene.app, widget, 0.0);
        scene.app.update();
        scene
            .app
            .world_mut()
            .commands()
            .set_tree(
                scene.panel,
                reset_slider_tree(plain_slider(0.25).value(0.8)),
            )
            .expect("rebuilt slider tree");
        scene.app.update();
        assert_close(slider_value(&scene.app, widget), 0.0);

        dispatch_click_with_jitter(&mut scene, widget, Vec2::new(2.0, 1.0));
        assert_close(slider_value(&scene.app, widget), 0.0);
        dispatch_click_with_jitter(&mut scene, widget, Vec2::new(1.0, 2.0));

        assert_close(slider_value(&scene.app, widget), 0.25);
        assert!(!dragging(&scene, widget));
        assert!(dispatch_empty(&scene));
        let proposals = &scene.app.world().resource::<RecordedProposals>().0;
        let (_, value, is_final, source) =
            *proposals.last().expect("reset proposal should be recorded");
        assert_close(value, 0.25);
        assert!(is_final);
        assert_eq!(source, SliderProposalSource::Pointer(DISPATCH_POINTER));
    }

    #[test]
    fn dispatched_double_click_started_on_track_stays_at_pointer_value() {
        let slider = plain_slider(0.25);
        let mut scene = dispatch_scene(reset_slider_tree(slider));
        scene.app.add_observer(slider_self_update);
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        add_pointer(&mut scene, DISPATCH_POINTER);

        dispatch_click(&mut scene, widget);
        assert_close(slider_value(&scene.app, widget), 0.0);
        dispatch_click(&mut scene, widget);

        assert_close(slider_value(&scene.app, widget), 0.0);
        assert!(!dragging(&scene, widget));
        assert!(dispatch_empty(&scene));
    }

    #[test]
    fn dispatched_release_reconciled_after_hover_lost() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);

        feed_hit(&mut scene, DISPATCH_POINTER, camera, widget);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            center(),
            PointerAction::Press(PointerButton::Primary),
        );
        scene.app.update();
        // Two frames without a hit leave the previous hover empty, so no
        // targeted release fires and the reconciler must resolve the raw
        // release itself.
        scene.app.update();
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            past_right(),
            PointerAction::Release(PointerButton::Primary),
        );
        scene.app.update();

        assert_eq!(
            dispatch_lifecycle(&scene),
            [
                Lifecycle::Grabbed,
                Lifecycle::Proposal { is_final: false },
                Lifecycle::Proposal { is_final: true },
                Lifecycle::Released,
            ]
        );
        assert!(!dragging(&scene, widget));
        assert!(dispatch_empty(&scene));
    }

    #[test]
    fn dispatched_pointer_removal_cancels_the_active_drag() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        let pointer_entity = add_pointer(&mut scene, DISPATCH_POINTER);

        feed_hit(&mut scene, DISPATCH_POINTER, camera, widget);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            center(),
            PointerAction::Press(PointerButton::Primary),
        );
        scene.app.update();
        assert!(dragging(&scene, widget));

        scene.app.world_mut().entity_mut(pointer_entity).despawn();
        scene.app.world_mut().flush();

        assert_eq!(
            dispatch_lifecycle(&scene).last(),
            Some(&Lifecycle::Canceled(SliderCancelCause::PointerRemoved))
        );
        assert!(dispatch_empty(&scene));
    }

    #[test]
    fn raw_same_slider_release_then_press_reuses_capture() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);

        set_hover(&mut scene, DISPATCH_POINTER, &[widget], &[widget], camera);
        run_raw(
            &mut scene,
            &[(
                DISPATCH_POINTER,
                center(),
                PointerAction::Press(PointerButton::Primary),
            )],
        );
        set_hover(&mut scene, DISPATCH_POINTER, &[widget], &[widget], camera);
        run_raw(
            &mut scene,
            &[
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Release(PointerButton::Primary),
                ),
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Press(PointerButton::Primary),
                ),
            ],
        );

        // The terminal frees occupancy first, then the reprojected press
        // recaptures the same slider for the same pointer within the batch.
        assert_eq!(
            dispatch_lifecycle(&scene),
            [
                Lifecycle::Grabbed,
                Lifecycle::Proposal { is_final: false },
                Lifecycle::Proposal { is_final: true },
                Lifecycle::Released,
                Lifecycle::Grabbed,
                Lifecycle::Proposal { is_final: false },
            ]
        );
        assert!(dragging(&scene, widget));
        assert_eq!(owner_of(&scene, widget), Some(DISPATCH_POINTER));
        assert!(!scene.app.world().resource::<SliderCaptures>().has_pending());
    }

    #[test]
    fn pointer_pending_release_and_semantic_adjustment_each_propose() {
        let mut scene = ltr_scene();
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        drag(&mut scene, CAPTURE_VIEWPORT * 0.5 + Vec2::new(40.0, 0.0));
        release(&mut scene, CAPTURE_VIEWPORT * 0.5 + Vec2::new(40.0, 0.0));
        assert_eq!(scene.app.world().resource::<RecordedProposals>().0.len(), 3);
        let widget = scene.widget;
        scene.app.world_mut().entity_mut(widget).insert(
            SliderState::new(
                SliderRange::new(0.0, f32::MAX).expect("wide range"),
                f32::MAX,
                SliderStepping::Stepped(SliderStep::new(f32::MAX).expect("wide step")),
                SliderDirection::LeftToRight,
            )
            .expect("wide slider state"),
        );
        scene.app.world_mut().trigger(RequestSliderAdjustment {
            entity:     widget,
            adjustment: SliderAdjustment::RelativeSteps(2.0),
        });
        scene.app.world_mut().flush();
        let proposals = &scene.app.world().resource::<RecordedProposals>().0;
        assert_eq!(proposals.len(), 4);
        assert!(proposals[3].1.is_finite());
        assert!(proposals[3].1 > f32::MAX / 2.0);

        let mut pending = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut pending.app, pending.panel, "level");
        let camera = pending.camera;
        add_pointer(&mut pending, DISPATCH_POINTER);
        set_hover(&mut pending, DISPATCH_POINTER, &[widget], &[widget], camera);
        run_raw(
            &mut pending,
            &[(
                DISPATCH_POINTER,
                center(),
                PointerAction::Press(PointerButton::Primary),
            )],
        );
        set_hover(&mut pending, DISPATCH_POINTER, &[widget], &[widget], camera);
        run_raw(
            &mut pending,
            &[
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Release(PointerButton::Primary),
                ),
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Press(PointerButton::Primary),
                ),
            ],
        );
        assert_eq!(
            pending.app.world().resource::<RecordedProposals>().0.len(),
            3,
            "the pending press must emit after the release frees the widget",
        );
    }

    #[test]
    fn raw_release_then_second_pointer_claims_freed_slider() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);
        add_pointer(&mut scene, SECOND_POINTER);

        set_hover(&mut scene, DISPATCH_POINTER, &[widget], &[widget], camera);
        run_raw(
            &mut scene,
            &[(
                DISPATCH_POINTER,
                center(),
                PointerAction::Press(PointerButton::Primary),
            )],
        );
        set_hover(&mut scene, DISPATCH_POINTER, &[widget], &[widget], camera);
        set_hover(&mut scene, SECOND_POINTER, &[], &[widget], camera);
        run_raw(
            &mut scene,
            &[
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Release(PointerButton::Primary),
                ),
                (
                    SECOND_POINTER,
                    center(),
                    PointerAction::Press(PointerButton::Primary),
                ),
            ],
        );

        assert_eq!(
            dispatch_lifecycle(&scene),
            [
                Lifecycle::Grabbed,
                Lifecycle::Proposal { is_final: false },
                Lifecycle::Proposal { is_final: true },
                Lifecycle::Released,
                Lifecycle::Grabbed,
                Lifecycle::Proposal { is_final: false },
            ]
        );
        assert!(dragging(&scene, widget));
        assert_eq!(owner_of(&scene, widget), Some(SECOND_POINTER));
    }

    #[test]
    fn raw_other_pointer_press_before_release_stays_rejected() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);
        add_pointer(&mut scene, SECOND_POINTER);

        set_hover(&mut scene, DISPATCH_POINTER, &[widget], &[widget], camera);
        run_raw(
            &mut scene,
            &[(
                DISPATCH_POINTER,
                center(),
                PointerAction::Press(PointerButton::Primary),
            )],
        );
        // The owner's previous hover is cleared, so its release routes through
        // the reconciler and frees the widget only after the batch is scanned.
        set_hover(&mut scene, DISPATCH_POINTER, &[], &[widget], camera);
        set_hover(&mut scene, SECOND_POINTER, &[], &[widget], camera);
        run_raw(
            &mut scene,
            &[
                (
                    SECOND_POINTER,
                    center(),
                    PointerAction::Press(PointerButton::Primary),
                ),
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Release(PointerButton::Primary),
                ),
            ],
        );

        // The competing press arrived before the terminal that freed the
        // widget, so it stays rejected and the owner still terminalizes once.
        assert_eq!(
            dispatch_lifecycle(&scene),
            [
                Lifecycle::Grabbed,
                Lifecycle::Proposal { is_final: false },
                Lifecycle::Proposal { is_final: true },
                Lifecycle::Released,
            ]
        );
        assert!(!dragging(&scene, widget));
        assert!(dispatch_empty(&scene));
    }

    #[test]
    fn raw_button_to_slider_handoff() {
        let mut scene = dispatch_scene(button_and_slider_tree());
        record_buttons(&mut scene);
        let button = resolve_widget(&mut scene.app, scene.panel, "act");
        let slider = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);

        set_hover(&mut scene, DISPATCH_POINTER, &[button], &[button], camera);
        run_raw(
            &mut scene,
            &[(
                DISPATCH_POINTER,
                center(),
                PointerAction::Press(PointerButton::Primary),
            )],
        );
        set_hover(&mut scene, DISPATCH_POINTER, &[button], &[slider], camera);
        run_raw(
            &mut scene,
            &[
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Release(PointerButton::Primary),
                ),
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Press(PointerButton::Primary),
                ),
            ],
        );

        assert!(scene.app.world().get::<ButtonPress>(button).is_none());
        assert!(dragging(&scene, slider));
        assert_eq!(owner_of(&scene, slider), Some(DISPATCH_POINTER));
        assert_eq!(owner_of(&scene, button), None);
        assert_eq!(
            scene.app.world().resource::<ButtonLog>().0,
            ["pressed", "released", "clicked"]
        );
        assert!(dispatch_lifecycle(&scene).contains(&Lifecycle::Grabbed));
    }

    #[test]
    fn raw_slider_to_button_handoff() {
        let mut scene = dispatch_scene(button_and_slider_tree());
        record_buttons(&mut scene);
        let button = resolve_widget(&mut scene.app, scene.panel, "act");
        let slider = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);

        set_hover(&mut scene, DISPATCH_POINTER, &[slider], &[slider], camera);
        run_raw(
            &mut scene,
            &[(
                DISPATCH_POINTER,
                center(),
                PointerAction::Press(PointerButton::Primary),
            )],
        );
        set_hover(&mut scene, DISPATCH_POINTER, &[slider], &[button], camera);
        run_raw(
            &mut scene,
            &[
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Release(PointerButton::Primary),
                ),
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Press(PointerButton::Primary),
                ),
            ],
        );

        assert!(!dragging(&scene, slider));
        assert!(scene.app.world().get::<ButtonPress>(button).is_some());
        assert_eq!(owner_of(&scene, button), Some(DISPATCH_POINTER));
        assert_eq!(owner_of(&scene, slider), None);
        assert_eq!(scene.app.world().resource::<ButtonLog>().0, ["pressed"]);
        assert_eq!(
            dispatch_lifecycle(&scene).last(),
            Some(&Lifecycle::Released)
        );
    }

    #[test]
    fn raw_cancel_then_press_ignores_the_later_action() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);

        set_hover(&mut scene, DISPATCH_POINTER, &[widget], &[widget], camera);
        run_raw(
            &mut scene,
            &[(
                DISPATCH_POINTER,
                center(),
                PointerAction::Press(PointerButton::Primary),
            )],
        );
        // Clear the hover so the reconciler owns the terminal, then a raw cancel
        // followed by a press: the cancel is terminal and the press is ignored.
        set_hover(&mut scene, DISPATCH_POINTER, &[], &[], camera);
        run_raw(
            &mut scene,
            &[
                (DISPATCH_POINTER, center(), PointerAction::Cancel),
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Press(PointerButton::Primary),
                ),
            ],
        );

        assert_eq!(
            dispatch_lifecycle(&scene),
            [
                Lifecycle::Grabbed,
                Lifecycle::Proposal { is_final: false },
                Lifecycle::Canceled(SliderCancelCause::PointerCanceled),
            ]
        );
        assert!(!dragging(&scene, widget));
        assert!(dispatch_empty(&scene));
    }

    #[test]
    fn raw_exhausted_sequence_preserves_the_owner() {
        let mut scene = dispatch_scene(two_slider_tree());
        let first = resolve_widget(&mut scene.app, scene.panel, "first");
        let second = resolve_widget(&mut scene.app, scene.panel, "second");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);
        add_pointer(&mut scene, SECOND_POINTER);

        set_hover(&mut scene, DISPATCH_POINTER, &[first], &[first], camera);
        run_raw(
            &mut scene,
            &[(
                DISPATCH_POINTER,
                center(),
                PointerAction::Press(PointerButton::Primary),
            )],
        );
        scene
            .app
            .world_mut()
            .resource_mut::<WidgetCaptures>()
            .saturate_sequence();
        set_hover(&mut scene, SECOND_POINTER, &[second], &[second], camera);
        run_raw(
            &mut scene,
            &[(
                SECOND_POINTER,
                center(),
                PointerAction::Press(PointerButton::Primary),
            )],
        );

        // The exhausted sequence rejects the new press and leaves the earlier
        // owner untouched.
        assert_eq!(owner_of(&scene, first), Some(DISPATCH_POINTER));
        assert!(dragging(&scene, first));
        assert_eq!(owner_of(&scene, second), None);
        assert!(!dragging(&scene, second));
    }

    fn past_left() -> Vec2 { CAPTURE_VIEWPORT.mul_add(Vec2::splat(0.5), Vec2::new(-400.0, 0.0)) }

    #[test]
    fn raw_release_terminalizes_despite_later_unobserved_press() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);
        let bogus_camera = scene.app.world_mut().spawn_empty().id();

        // Accepted press through the real dispatcher: observed, captured,
        // dragging. Only `pointer_events` runs, so the reconciler has not yet
        // drained this observation.
        set_hover(&mut scene, DISPATCH_POINTER, &[widget], &[widget], camera);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            center(),
            PointerAction::Press(PointerButton::Primary),
        );
        let result = scene.app.world_mut().run_system_cached(pointer_events);
        assert!(result.is_ok());
        assert!(dragging(&scene, widget));
        assert_eq!(owner_of(&scene, widget), Some(DISPATCH_POINTER));

        // The same retained unread window carries the real release and then a
        // later primary raw press. The empty previous hover keeps the release
        // out of Bevy's targeted path so the reconciler owns it; the current
        // hover names a camera that cannot project, so when the shipped
        // `pointer_events` delivers the later press, `grab_from_pointer`
        // receives it and rejects it on projection failure before `observe_press`
        // — a real unobserved press that raised the raw count without entering
        // the widget capture-order path.
        set_hover(&mut scene, DISPATCH_POINTER, &[], &[widget], bogus_camera);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            center(),
            PointerAction::Release(PointerButton::Primary),
        );
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            center(),
            PointerAction::Press(PointerButton::Primary),
        );
        let result = scene.app.world_mut().run_system_cached(pointer_events);
        assert!(result.is_ok());
        let result = scene
            .app
            .world_mut()
            .run_system_cached(crate::widgets::interaction::reconcile_pointer_input);
        assert!(result.is_ok());
        scene.app.world_mut().flush();

        // The later press reached `grab_from_pointer` yet claimed and observed
        // nothing, so no pending recapture is recorded.
        assert!(!scene.app.world().resource::<SliderCaptures>().has_pending());

        // The accepted drag terminalizes after its real release and frees
        // occupancy; the later unobserved press does not defer it.
        assert_eq!(
            dispatch_lifecycle(&scene),
            [
                Lifecycle::Grabbed,
                Lifecycle::Proposal { is_final: false },
                Lifecycle::Proposal { is_final: true },
                Lifecycle::Released,
            ]
        );
        assert!(!dragging(&scene, widget));
        assert!(dispatch_empty(&scene));
    }

    #[test]
    fn raw_release_finalizes_widget_with_missing_lookup_components() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);

        // Live capture through the real dispatcher.
        set_hover(&mut scene, DISPATCH_POINTER, &[widget], &[widget], camera);
        run_raw(
            &mut scene,
            &[(
                DISPATCH_POINTER,
                center(),
                PointerAction::Press(PointerButton::Primary),
            )],
        );
        assert!(dragging(&scene, widget));
        assert_eq!(owner_of(&scene, widget), Some(DISPATCH_POINTER));

        // The widget loses its lookup components while its shared occupancy and
        // typed slider payload survive, so the reconciler's `WidgetKind` lookup
        // returns `None`.
        scene
            .app
            .world_mut()
            .entity_mut(widget)
            .remove::<WidgetOf>();
        scene.app.world_mut().flush();

        // A raw release the reconciler must resolve, followed by a later press
        // in the same window.
        set_hover(&mut scene, DISPATCH_POINTER, &[], &[], camera);
        run_raw(
            &mut scene,
            &[
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Release(PointerButton::Primary),
                ),
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Press(PointerButton::Primary),
                ),
            ],
        );

        // The lost widget is finalized through its surviving typed payload
        // exactly once and shared occupancy frees; the reconciler never treated
        // the pointer as freed while no terminal ran.
        assert_eq!(
            dispatch_lifecycle(&scene).last(),
            Some(&Lifecycle::Canceled(SliderCancelCause::CaptureLost))
        );
        assert!(!dragging(&scene, widget));
        assert!(dispatch_empty(&scene));
    }

    #[test]
    fn raw_cancel_finalizes_lost_widget_with_pointer_canceled_cause() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);

        set_hover(&mut scene, DISPATCH_POINTER, &[widget], &[widget], camera);
        run_raw(
            &mut scene,
            &[(
                DISPATCH_POINTER,
                center(),
                PointerAction::Press(PointerButton::Primary),
            )],
        );
        assert!(dragging(&scene, widget));

        // The widget loses its lookup components so the reconciler's `WidgetKind`
        // lookup returns `None`, while its shared occupancy and typed slider
        // payload survive.
        scene
            .app
            .world_mut()
            .entity_mut(widget)
            .remove::<WidgetOf>();
        scene.app.world_mut().flush();

        // A raw cancel the reconciler must resolve. The cleared hover keeps the
        // targeted `PointerCancel` from firing, so the lost-widget fallback
        // owns it and must preserve the concrete `PointerCanceled` cause rather
        // than degrading a cancel to `CaptureLost`.
        set_hover(&mut scene, DISPATCH_POINTER, &[], &[], camera);
        run_raw(
            &mut scene,
            &[(DISPATCH_POINTER, center(), PointerAction::Cancel)],
        );

        assert_eq!(
            dispatch_lifecycle(&scene).last(),
            Some(&Lifecycle::Canceled(SliderCancelCause::PointerCanceled))
        );
        assert!(!dragging(&scene, widget));
        assert!(dispatch_empty(&scene));
    }

    #[test]
    fn raw_release_with_present_kind_and_missing_payload_frees_nothing() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);

        set_hover(&mut scene, DISPATCH_POINTER, &[widget], &[widget], camera);
        run_raw(
            &mut scene,
            &[(
                DISPATCH_POINTER,
                center(),
                PointerAction::Press(PointerButton::Primary),
            )],
        );
        assert!(dragging(&scene, widget));
        assert_eq!(owner_of(&scene, widget), Some(DISPATCH_POINTER));
        let baseline = dispatch_lifecycle(&scene);

        // Drop the typed slider payload while the `WidgetKind` lookup and shared
        // occupancy remain: `resolve_release` finds no captured payload to
        // reproject.
        assert!(
            scene
                .app
                .world_mut()
                .resource_mut::<SliderCaptures>()
                .take(widget)
                .is_some()
        );

        // A raw release the reconciler owns (empty previous hover) followed by a
        // later press delivered to the still-occupied widget. Because the release
        // records no terminal, the reconciler must not mark the pointer freed, so
        // the later press cannot claim the widget.
        set_hover(&mut scene, DISPATCH_POINTER, &[], &[widget], camera);
        run_raw(
            &mut scene,
            &[
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Release(PointerButton::Primary),
                ),
                (
                    DISPATCH_POINTER,
                    center(),
                    PointerAction::Press(PointerButton::Primary),
                ),
            ],
        );

        // No terminal ran, occupancy is retained, and the later press claimed
        // nothing.
        assert_eq!(dispatch_lifecycle(&scene), baseline);
        assert!(dragging(&scene, widget));
        assert_eq!(owner_of(&scene, widget), Some(DISPATCH_POINTER));
        assert!(!scene.app.world().resource::<SliderCaptures>().has_pending());
    }

    #[test]
    fn dispatched_moves_reproject_and_replace_raw_target() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);

        // Press to capture, then feed raw `Move` actions so Bevy's dispatcher
        // delivers real `DragStart` / `Drag` and the slider reprojects each
        // position through `pointer_events`.
        feed_hit(&mut scene, DISPATCH_POINTER, camera, widget);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            center(),
            PointerAction::Press(PointerButton::Primary),
        );
        scene.app.update();
        assert!(dragging(&scene, widget));

        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            past_right(),
            PointerAction::Move {
                delta: past_right() - center(),
            },
        );
        scene.app.update();
        let after_right = scene
            .app
            .world()
            .resource::<SliderCaptures>()
            .latest_raw_target(widget)
            .expect("a live drag stores a raw target");

        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            past_left(),
            PointerAction::Move {
                delta: past_left() - past_right(),
            },
        );
        scene.app.update();
        let after_left = scene
            .app
            .world()
            .resource::<SliderCaptures>()
            .latest_raw_target(widget)
            .expect("a live drag stores a raw target");

        // The grab plus two real drags each emit exactly one non-final proposal.
        assert_eq!(
            dispatch_lifecycle(&scene),
            [
                Lifecycle::Grabbed,
                Lifecycle::Proposal { is_final: false },
                Lifecycle::Proposal { is_final: false },
                Lifecycle::Proposal { is_final: false },
            ]
        );
        // Two opposite drags replace the stored raw target with different
        // values.
        assert_close(after_right, 1.0);
        assert_close(after_left, 0.0);
        assert!(after_right > after_left);
        assert!(dragging(&scene, widget));
    }

    #[test]
    fn dispatched_move_projection_loss_cancels_once_with_later_dragend() {
        let mut scene = dispatch_scene(slider_tree("level", slider0()));
        let widget = resolve_widget(&mut scene.app, scene.panel, "level");
        let camera = scene.camera;
        add_pointer(&mut scene, DISPATCH_POINTER);

        feed_hit(&mut scene, DISPATCH_POINTER, camera, widget);
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            center(),
            PointerAction::Press(PointerButton::Primary),
        );
        scene.app.update();
        assert!(dragging(&scene, widget));

        // The captured camera is lost, so the next real drag cannot project.
        // The drag cancels exactly once even though the release still delivers a
        // later `DragEnd` for the same pointer.
        scene.app.world_mut().entity_mut(camera).despawn();
        scene.app.world_mut().flush();

        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            past_right(),
            PointerAction::Move {
                delta: past_right() - center(),
            },
        );
        feed_input(
            &mut scene,
            DISPATCH_POINTER,
            past_right(),
            PointerAction::Release(PointerButton::Primary),
        );
        scene.app.update();

        let canceled = dispatch_lifecycle(&scene)
            .iter()
            .filter(|event| matches!(event, Lifecycle::Canceled(_)))
            .count();
        assert_eq!(canceled, 1);
        assert_eq!(
            dispatch_lifecycle(&scene).last(),
            Some(&Lifecycle::Canceled(
                SliderCancelCause::ProjectionUnavailable
            ))
        );
        assert!(!dragging(&scene, widget));
        assert!(dispatch_empty(&scene));
    }

    #[test]
    fn picking_without_interaction_skips_slider_reconciliation() {
        let mut app = test_app();
        app.set_error_handler(bevy::ecs::error::panic)
            .insert_resource(PickingSettings {
                is_enabled: false,
                ..default()
            })
            .add_plugins(PickingPlugin);
        let _panel = spawn_panel(&mut app, slider_tree("level", slider0()));

        app.update();

        // Without `InteractionPlugin` the hover state never exists, so the
        // reconciler's run condition is never met.
        assert!(!app.world().contains_resource::<PointerState>());
        assert!(!app.world().contains_resource::<HoverMap>());
    }

    #[test]
    fn same_id_refresh_preserves_live_drag_and_applied_value() {
        let mut scene = ltr_scene();
        press(&mut scene, CAPTURE_VIEWPORT * 0.5);
        assert!(is_dragging(&scene));
        let applied = scene
            .app
            .world_mut()
            .get_mut::<SliderState>(scene.widget)
            .expect("widget should carry slider state")
            .set_value(0.75);
        assert_eq!(applied, Ok(true));

        // Re-author the identical slider: same panel, id, and kind. The refresh
        // preserves the capture and the live applied value.
        let panel = scene.panel;
        scene
            .app
            .world_mut()
            .commands()
            .set_tree(panel, slider_tree("level", slider0()))
            .expect("same-id refresh should be accepted");
        scene.app.update();

        assert_eq!(resolve_widget(&mut scene.app, panel, "level"), scene.widget);
        assert!(is_dragging(&scene), "the refresh preserves the live drag");
        assert_close(applied_value(&scene), 0.75);
        assert!(!captures_empty(&scene));
        assert_eq!(
            lifecycle(&scene),
            [Lifecycle::Grabbed, Lifecycle::Proposal { is_final: false }]
        );
    }

    #[test]
    fn built_in_escape_cancels_the_active_slider_drag() {
        let mut app = capture_app();
        app.add_plugins((InputPlugin, WidgetInputPlugin));
        let window = app
            .world_mut()
            .spawn((
                Window {
                    focused: true,
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = app
            .world_mut()
            .spawn((
                seeded_camera(),
                seeded_camera_transform(),
                GlobalTransform::from(seeded_camera_transform()),
                RenderTarget::Window(WindowRef::Primary),
            ))
            .id();
        app.finish();
        let panel = projecting_panel(&mut app, slider_tree("level", slider0()));
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let target = RenderTarget::Window(WindowRef::Primary)
            .normalize(Some(window))
            .expect("primary window target normalizes");

        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
        app.world_mut().trigger(PointerPress {
            entity:  widget,
            pointer: Pointer::new(
                CAPTURE_POINTER,
                Location {
                    target,
                    position: CAPTURE_VIEWPORT * 0.5,
                },
            ),
            button:  PointerButton::Primary,
            hit:     HitData::new(camera, 0.0, None, None),
            count:   1,
        });
        app.world_mut().flush();
        assert!(app.world().get::<SliderDrag>(widget).is_some());

        // The built-in Escape binding reaches the same explicit-cancel path as
        // `SemanticWidgetIntent::Cancel`.
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::Escape,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window,
        });
        app.update();

        assert_eq!(
            app.world().resource::<RecordedLifecycle>().0.last(),
            Some(&Lifecycle::Canceled(SliderCancelCause::Explicit))
        );
        assert!(app.world().resource::<SliderCaptures>().is_empty());
        assert!(app.world().resource::<WidgetCaptures>().is_empty());
    }

    #[test]
    fn slider_hit_area_is_unchanged() {
        let close = |actual: f32, expected: f32| {
            assert!(
                (actual - expected).abs() < 1e-4,
                "expected {expected}, got {actual}"
            );
        };
        for label in [9.0, 13.0, 26.0] {
            let metrics = crate::ControlMetrics::from_label_size(label);
            let mut builder = crate::LayoutBuilder::new(400.0, 100.0);
            builder.slider(
                "level",
                plain_slider(0.5),
                &metrics,
                SliderReachableBand::Hidden,
            );
            let mut app = test_app();
            // Solved slots are in points; keep the fixture's bare lengths in points too.
            let panel = DiegeticPanel::world()
                .size(crate::Pt(400.0), crate::Pt(100.0))
                .with_tree(builder.build())
                .build()
                .expect("slider panel");
            let panel = app.world_mut().spawn(panel).id();
            app.update();
            let widget = resolve_widget(&mut app, panel, "level");
            let slots = slots_of(&app, widget);
            let root = slots
                .content_box(VisualSlotId::SLIDER_ROOT)
                .expect("slider press box");
            close(root.width, label * 133.0 / 13.0);
            close(root.height, label * 16.0 / 13.0);
            let mut state = app
                .world()
                .get::<SliderState>(widget)
                .expect("slider state")
                .clone();
            for value in [0.0, 0.5, 1.0] {
                state.set_value(value).expect("in-range value");
                let hit = current_thumb_hit_bounds(&slots, &state).expect("thumb drag box");
                let width = label * 20.0 / 13.0;
                let height = label * 16.0 / 13.0;
                close(hit.width, THUMB_HIT_SLOP_POINTS.mul_add(2.0, width));
                close(hit.height, THUMB_HIT_SLOP_POINTS.mul_add(2.0, height));
                close(
                    hit.x,
                    value.mul_add(root.width - width, root.x) - THUMB_HIT_SLOP_POINTS,
                );
                close(hit.y, root.y - THUMB_HIT_SLOP_POINTS);
                let thumb = slots
                    .border_box(VisualSlotId::SLIDER_THUMB)
                    .expect("painted thumb");
                let offset = projection::thumb_translation(&slots, &state).expect("thumb position");
                close(thumb.center().0 + offset.x, hit.center().0);
                close(thumb.center().1 + offset.y, hit.center().1);
            }
        }
    }

    #[test]
    fn thumb_hit_bounds_follow_the_presented_thumb_on_both_axes() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, thumb_slider_tree("level", plain_slider(0.5), 8.0));
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let slots = slots_of(&app, widget);
        let state = app
            .world()
            .get::<SliderState>(widget)
            .expect("widget should carry slider state");
        let bounds = current_thumb_hit_bounds(&slots, state).expect("thumb should have hit bounds");
        let center = Vec2::from(bounds.center());

        assert!(bounds.contains(center));
        assert!(!bounds.contains(Vec2::new(center.x, bounds.y - THUMB_HIT_SLOP_POINTS,)));
        assert!(!bounds.contains(Vec2::new(bounds.x - THUMB_HIT_SLOP_POINTS, center.y,)));
    }
}
