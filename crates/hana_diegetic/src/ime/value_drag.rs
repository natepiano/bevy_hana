//! Screen-distance number-field drags, committed through the ordinary IME path.

use bevy::camera::NormalizedRenderTarget;
use bevy::picking::PickingSystems;
use bevy::picking::pointer::PointerAction;
use bevy::picking::pointer::PointerId;
use bevy::picking::pointer::PointerInput;
use bevy::prelude::*;
use bevy::window::CursorIcon;
use bevy::window::SystemCursorIcon;

use super::ImeAuthoredFieldText;
use super::ImeCommitCause;
use super::ImeEditableFieldSpec;
use super::ImeOpenSession;
use super::ImeRequestCommit;
use super::ImeTarget;
use super::PanelElementId;
use super::activation;
use super::activation::ImeFieldAuthoringLock;
use super::pointer::VALUE_DRAG_THRESHOLD_PX;
use super::session::ActiveImeSession;
use crate::ComputedDiegeticPanel;
use crate::DiegeticPanel;
use crate::FineAdjustment;
use crate::widgets::PanelWidget;
use crate::widgets::WidgetDisabled;
use crate::widgets::WidgetKind;
use crate::widgets::WidgetOf;

/// Logical screen pixels needed to traverse a number field's full range.
const VALUE_DRAG_RANGE_PX: f64 = 200.0;

pub(super) fn install(app: &mut App) {
    if !app.is_plugin_added::<crate::FineAdjustmentPlugin>() {
        app.add_plugins(crate::FineAdjustmentPlugin);
    }
    app.init_resource::<FieldValueDrag>()
        .add_observer(press)
        .add_observer(drag)
        .add_observer(end)
        .add_observer(release)
        .add_observer(cancel)
        .add_systems(
            PreUpdate,
            (cleanup, reanchor, escape)
                .chain()
                .after(crate::FineAdjustmentSystems)
                .before(PickingSystems::ProcessInput),
        )
        .add_systems(
            PreUpdate,
            finish_raw_input
                .after(PickingSystems::Last)
                .run_if(resource_exists::<Messages<PointerInput>>),
        );
}

/// Enables vertical value dragging for one named numeric field on a panel.
///
/// Updates use the field's existing IME commit validator and writer. Attach
/// this component to the owning panel alongside its authored field text.
#[derive(Component, Clone, Debug, PartialEq)]
pub enum ImeNumberFieldDrag {
    /// A floating-point field with inclusive finite, ordered bounds.
    Float {
        /// Panel-local field identity.
        field_id: PanelElementId,
        /// Inclusive lower bound.
        min:      f32,
        /// Inclusive upper bound.
        max:      f32,
    },
    /// An integer field; proposals round to the nearest whole value.
    Integer {
        /// Panel-local field identity.
        field_id: PanelElementId,
        /// Inclusive lower bound.
        min:      i64,
        /// Inclusive upper bound.
        max:      i64,
    },
}

impl ImeNumberFieldDrag {
    const fn field_id(&self) -> &PanelElementId {
        match self {
            Self::Float { field_id, .. } | Self::Integer { field_id, .. } => field_id,
        }
    }

    // Screen motion is approximate; the commit path retains exact integer limits.
    #[allow(
        clippy::cast_precision_loss,
        reason = "pointer motion cannot resolve 64-bit integer precision"
    )]
    fn bounds(&self) -> (f64, f64) {
        match *self {
            Self::Float { min, max, .. } => (f64::from(min), f64::from(max)),
            Self::Integer { min, max, .. } => (min as f64, max as f64),
        }
    }

    fn proposal_text(&self, value: f64) -> String {
        match self {
            Self::Float { .. } => value.to_string(),
            Self::Integer { .. } => format!("{:.0}", value.round()),
        }
    }
}

/// Presentation marker on the editable widget while its number is dragged.
#[derive(Component)]
pub(crate) struct ImeValueDragging;

#[derive(Resource, Default)]
pub(super) struct FieldValueDrag {
    active: DragCapture,
}

#[derive(Default)]
#[allow(
    clippy::large_enum_variant,
    reason = "the single resource stores one capture in place to avoid an extra allocation per press"
)]
enum DragCapture {
    #[default]
    Idle,
    Captured(ValueDrag),
}

enum CursorRestore {
    RemoveOverride,
    RestoreOverride(CursorIcon),
}

enum CaptureCompletion {
    KeepCaptured,
    ReleaseCaptured,
}

impl FieldValueDrag {
    pub(super) fn captures(&self, pointer: PointerId, entity: Entity) -> bool {
        matches!(&self.active, DragCapture::Captured(drag)
            if drag.pointer == pointer && drag.entity == entity)
    }

    pub(super) fn suppresses_click(&self, pointer: PointerId, entity: Entity) -> bool {
        matches!(&self.active, DragCapture::Captured(drag)
            if drag.pointer == pointer
                && drag.entity == entity
                && !matches!(drag.phase, DragPhase::Armed)
        )
    }

    pub(super) fn suppresses_editor(&self, target: &ImeTarget) -> bool {
        matches!(&self.active, DragCapture::Captured(drag)
            if drag.target == *target && !matches!(drag.phase, DragPhase::Armed))
    }

    fn finish(&mut self, commands: &mut Commands) {
        if let DragCapture::Captured(drag) = std::mem::take(&mut self.active) {
            drag.clear_presentation(commands);
        }
    }
}

enum DragPhase {
    Armed,
    Dragging {
        anchor_value:    f64,
        anchor_y:        f32,
        previous_cursor: CursorRestore,
    },
    Canceled,
}

struct ValueDrag {
    pointer:        PointerId,
    entity:         Entity,
    panel:          Entity,
    window:         Entity,
    target:         ImeTarget,
    spec:           ImeEditableFieldSpec,
    range:          ImeNumberFieldDrag,
    original_text:  String,
    original_value: f64,
    press_y:        f32,
    last_y:         f32,
    adjustment:     FineAdjustment,
    phase:          DragPhase,
}

impl ValueDrag {
    fn commit(&self, text: String, commands: &mut Commands) {
        let request = ImeOpenSession {
            window:       self.window,
            target:       self.target.clone(),
            initial_text: text,
            field_spec:   self.spec.clone(),
            anchor:       None,
        };
        commands.queue(move |world: &mut World| {
            world.trigger(request);
            world.flush();
            if let Some(session_id) = world.resource::<ActiveImeSession>().active_session_id() {
                world.trigger(ImeRequestCommit {
                    session_id,
                    cause: ImeCommitCause::Request,
                });
            }
        });
    }

    fn clear_presentation(&self, commands: &mut Commands) {
        commands
            .entity(self.entity)
            .try_remove::<ImeValueDragging>();
        if let DragPhase::Dragging {
            previous_cursor, ..
        } = &self.phase
        {
            match previous_cursor {
                CursorRestore::RestoreOverride(cursor) => {
                    commands.entity(self.window).try_insert(cursor.clone());
                },
                CursorRestore::RemoveOverride => {
                    commands.entity(self.window).try_remove::<CursorIcon>();
                },
            }
        }
    }
}

pub(super) fn press(
    press: On<PointerPress>,
    widgets: Query<(&PanelWidget, &WidgetKind, &WidgetOf, Has<WidgetDisabled>)>,
    panels: Query<(
        &DiegeticPanel,
        &ComputedDiegeticPanel,
        &ImeNumberFieldDrag,
        Option<&ImeAuthoredFieldText>,
        Has<ImeFieldAuthoringLock>,
    )>,
    adjustment: Res<FineAdjustment>,
    active: Res<ActiveImeSession>,
    mut capture: ResMut<FieldValueDrag>,
) {
    if press.button != PointerButton::Primary
        || matches!(capture.active, DragCapture::Captured(_))
        || active.is_composing()
    {
        return;
    }
    let Ok((widget, kind, owner, disabled)) = widgets.get(press.event_target()) else {
        return;
    };
    if *kind != WidgetKind::EditableField || disabled {
        return;
    }
    let Ok((panel, computed, range, authored, locked)) = panels.get(owner.panel()) else {
        return;
    };
    if locked || range.field_id() != widget.id() {
        return;
    }
    let Some(record) = computed
        .field_records()
        .iter()
        .find(|record| !record.duplicate_id && record.field_id == *widget.id())
    else {
        return;
    };
    let NormalizedRenderTarget::Window(window) = &press.pointer.target else {
        return;
    };
    let original_text = activation::initial_editor_text(record, authored);
    let Ok(original_value) = original_text.parse::<f64>() else {
        return;
    };
    let (min, max) = range.bounds();
    if !original_value.is_finite() || !min.is_finite() || !max.is_finite() || min >= max {
        return;
    }
    capture.active = DragCapture::Captured(ValueDrag {
        pointer: press.pointer.id,
        entity: press.event_target(),
        panel: owner.panel(),
        window: window.entity(),
        target: activation::panel_field_target(panel, owner.panel(), record.field_id.clone()),
        spec: record.field_spec.clone(),
        range: range.clone(),
        original_text,
        original_value,
        press_y: press.pointer.position.y,
        last_y: press.pointer.position.y,
        adjustment: *adjustment,
        phase: DragPhase::Armed,
    });
}

pub(super) fn drag(
    mut event: On<PointerDrag>,
    mut capture: ResMut<FieldValueDrag>,
    cursors: Query<&CursorIcon>,
    mut commands: Commands,
) {
    if event.button != PointerButton::Primary {
        return;
    }
    let DragCapture::Captured(drag) = &mut capture.active else {
        return;
    };
    if drag.entity != event.event_target() || drag.pointer != event.pointer.id {
        return;
    }
    let y = event.pointer.position.y;
    if !y.is_finite() {
        return;
    }
    let previous_y = drag.last_y;
    drag.last_y = y;
    if matches!(drag.phase, DragPhase::Armed) && (y - drag.press_y).abs() > VALUE_DRAG_THRESHOLD_PX
    {
        drag.phase = DragPhase::Dragging {
            anchor_value:    drag.original_value,
            anchor_y:        drag.press_y,
            previous_cursor: match cursors.get(drag.window) {
                Ok(cursor) => CursorRestore::RestoreOverride(cursor.clone()),
                Err(_) => CursorRestore::RemoveOverride,
            },
        };
        commands.entity(drag.entity).insert(ImeValueDragging);
        commands
            .entity(drag.window)
            .insert(CursorIcon::System(SystemCursorIcon::NsResize));
    }
    if let DragPhase::Dragging {
        anchor_value,
        anchor_y,
        ..
    } = drag.phase
        && (y - previous_y).abs() > f32::EPSILON
    {
        let (min, max) = drag.range.bounds();
        let fraction =
            f64::from(anchor_y - y) * f64::from(drag.adjustment.rate()) / VALUE_DRAG_RANGE_PX;
        drag.commit(
            drag.range
                .proposal_text(fraction.mul_add(max - min, anchor_value)),
            &mut commands,
        );
    }
    event.propagate(false);
}

/// Samples modifier changes before pointer dispatch, using the accepted value.
pub(super) fn reanchor(
    adjustment: Res<FineAdjustment>,
    authored: Query<&ImeAuthoredFieldText>,
    mut capture: ResMut<FieldValueDrag>,
) {
    let DragCapture::Captured(drag) = &mut capture.active else {
        return;
    };
    if drag.adjustment == *adjustment {
        return;
    }
    if let DragPhase::Dragging {
        anchor_value,
        anchor_y,
        ..
    } = &mut drag.phase
        && let Ok(text) = authored.get(drag.panel)
        && let Ok(value) = text.text().parse::<f64>()
    {
        *anchor_value = value;
        *anchor_y = drag.last_y;
    }
    drag.adjustment = *adjustment;
}

pub(super) fn end(
    end: On<PointerDragEnd>,
    mut capture: ResMut<FieldValueDrag>,
    mut commands: Commands,
) {
    if end.button == PointerButton::Primary && capture.captures(end.pointer.id, end.event_target())
    {
        capture.finish(&mut commands);
    }
}

pub(super) fn release(
    end: On<PointerRelease>,
    mut capture: ResMut<FieldValueDrag>,
    mut commands: Commands,
) {
    if end.button == PointerButton::Primary && capture.captures(end.pointer.id, end.event_target())
    {
        capture.finish(&mut commands);
    }
}

pub(super) fn cancel(
    cancel: On<PointerCancel>,
    mut capture: ResMut<FieldValueDrag>,
    mut commands: Commands,
) {
    if matches!(&capture.active, DragCapture::Captured(drag) if drag.pointer == cancel.pointer.id) {
        capture.finish(&mut commands);
    }
}

pub(super) fn escape(
    keys: Res<ButtonInput<KeyCode>>,
    mut capture: ResMut<FieldValueDrag>,
    mut commands: Commands,
) {
    if keys.just_pressed(KeyCode::Escape)
        && let DragCapture::Captured(drag) = &mut capture.active
        && matches!(drag.phase, DragPhase::Dragging { .. })
    {
        drag.commit(drag.original_text.clone(), &mut commands);
        drag.clear_presentation(&mut commands);
        drag.phase = DragPhase::Canceled;
    }
}

/// Clears captures whose field or window can no longer receive input.
fn cleanup(
    widgets: Query<(&WidgetKind, Has<WidgetDisabled>)>,
    panels: Query<(), (With<ImeNumberFieldDrag>, Without<ImeFieldAuthoringLock>)>,
    windows: Query<&Window>,
    mut capture: ResMut<FieldValueDrag>,
    mut commands: Commands,
) {
    let stale = matches!(&capture.active, DragCapture::Captured(drag)
        if !widgets
            .get(drag.entity)
            .is_ok_and(|(kind, disabled)| *kind == WidgetKind::EditableField && !disabled)
            || !panels.contains(drag.panel)
            || !windows.get(drag.window).is_ok_and(|window| window.focused)
    );
    if stale {
        capture.finish(&mut commands);
    }
}

/// Bevy need not send a targeted release when the pointer is off the field.
fn finish_raw_input(
    mut inputs: MessageReader<PointerInput>,
    mut capture: ResMut<FieldValueDrag>,
    mut commands: Commands,
) {
    let mut completion = CaptureCompletion::KeepCaptured;
    for input in inputs.read() {
        if matches!(&capture.active, DragCapture::Captured(drag) if drag.pointer == input.pointer_id)
        {
            match input.action {
                PointerAction::Press(PointerButton::Primary) => {
                    completion = CaptureCompletion::KeepCaptured;
                },
                PointerAction::Release(PointerButton::Primary) | PointerAction::Cancel => {
                    completion = CaptureCompletion::ReleaseCaptured;
                },
                _ => {},
            }
        }
    }
    if matches!(completion, CaptureCompletion::ReleaseCaptured) {
        capture.finish(&mut commands);
    }
}
