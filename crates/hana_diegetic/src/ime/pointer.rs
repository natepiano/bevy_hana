//! Pointer placement and range selection for inline fields.

use bevy::camera::NormalizedRenderTarget;
use bevy::camera::RenderTarget;
use bevy::ecs::entity::ContainsEntity;
use bevy::ecs::system::SystemParam;
use bevy::picking::backend::HitData;
use bevy::picking::pointer::Location;
use bevy::picking::pointer::PointerId;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::ImeAuthoredFieldText;
use super::ImeInputBlocker;
use super::ImeOpenSession;
use super::ImeTarget;
use super::activation;
use super::activation::ImeFieldAuthoringLock;
use super::buffer::ImeEditCommand;
use super::editor;
use super::editor::ImeBlurIntent;
use super::editor::ImeEditorState;
use super::editor::PendingImePanelAnchor;
use super::session::ActiveImeSession;
use super::value_drag::FieldValueDrag;
use crate::ComputedDiegeticPanel;
use crate::DiegeticPanel;
use crate::DiegeticTextMeasurer;
use crate::PanelFieldRecord;
use crate::render::CapturedCameraRay;
use crate::render::{self};
use crate::widgets::PanelWidget;
use crate::widgets::WidgetDisabled;
use crate::widgets::WidgetKind;
use crate::widgets::WidgetOf;

/// Vertical screen motion before a numeric press becomes a value drag.
pub(super) const VALUE_DRAG_THRESHOLD_PX: f32 = 4.0;

#[derive(SystemParam)]
pub(super) struct FieldPointer<'w, 's> {
    widgets: Query<
        'w,
        's,
        (
            &'static PanelWidget,
            &'static WidgetKind,
            &'static WidgetOf,
            Has<WidgetDisabled>,
        ),
    >,
    panels: Query<
        'w,
        's,
        (
            &'static DiegeticPanel,
            &'static ComputedDiegeticPanel,
            &'static GlobalTransform,
            Option<&'static ImeAuthoredFieldText>,
            Has<ImeFieldAuthoringLock>,
        ),
    >,
    active:   ResMut<'w, ActiveImeSession>,
    pending:  ResMut<'w, PendingImePanelAnchor>,
    blocker:  Res<'w, ImeInputBlocker>,
    measurer: Res<'w, DiegeticTextMeasurer>,
}

impl FieldPointer<'_, '_> {
    fn place(
        &mut self,
        entity: Entity,
        hit: &HitData,
        location: &Location,
        count: u8,
        commands: &mut Commands,
    ) -> Option<(ImeTarget, usize)> {
        let widget = self.widgets.get(entity).ok();
        if widget
            .is_some_and(|(_, kind, _, disabled)| *kind != WidgetKind::EditableField || disabled)
        {
            return None;
        }
        let panel_entity = widget.map_or(entity, |(_, _, owner, _)| owner.panel());
        let (panel, computed, transform, authored, locked) = self.panels.get(panel_entity).ok()?;
        if locked {
            return None;
        }
        let local = render::project_flat_panel_hit(hit.position?, panel, transform)?;
        let record = if let Some((widget, _, _, _)) = widget {
            computed
                .field_records()
                .iter()
                .find(|record| !record.duplicate_id && record.field_id == *widget.id())?
        } else {
            computed.field_at_local_position(local)?
        };
        let target = activation::panel_field_target(panel, panel_entity, record.field_id.clone());
        let already_editing = self.active.active_target() == Some(&target);
        if widget.is_none()
            && !already_editing
            && count < 2
            && !panel.activates_field_on_single_click()
        {
            return None;
        }
        let NormalizedRenderTarget::Window(window) = &location.target else {
            return None;
        };
        let window = window.entity();
        let initial_text = activation::initial_editor_text(record, authored);
        let text = if already_editing {
            self.active.active_text()?
        } else {
            &initial_text
        };
        let text_hit = editor::text_hit_at_x(
            text,
            local.x,
            if already_editing {
                horizontal_scroll(computed, record)
            } else {
                0.0
            },
            record,
            &self.measurer,
        );
        let command = editor::pointer_command(count, text_hit);
        if already_editing {
            if !self.active.is_composing()
                && let Some(changed) = self.active.apply_edit_command(command, &self.blocker)
            {
                commands.trigger(changed);
            }
        } else {
            self.pending.store(
                panel_entity,
                record.field_id.clone(),
                Some(hit.camera),
                window,
                record.bounds,
                record.presentation().clone(),
            );
            self.pending
                .set_pointer_edit(target.clone(), window, command);
            commands.trigger(ImeOpenSession {
                window,
                target: target.clone(),
                initial_text,
                field_spec: record.field_spec.clone(),
                anchor: None,
            });
        }
        Some((target, text_hit.insertion))
    }
}

fn horizontal_scroll(computed: &ComputedDiegeticPanel, record: &PanelFieldRecord) -> f32 {
    computed
        .layout()
        .solved()
        .and_then(|layout| {
            layout
                .scrolls
                .iter()
                .find(|scroll| scroll.element_index == record.element_index)
        })
        .map_or(0.0, |scroll| scroll.offset_points.x)
}

pub(super) fn click(
    mut click: On<PointerClick>,
    mut pointer: FieldPointer,
    editor: Res<ImeEditorState>,
    selection_drag: Res<FieldSelectionDrag>,
    value_drag: Res<FieldValueDrag>,
    mut blur: ResMut<ImeBlurIntent>,
    mut commands: Commands,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    if editor.is_overlay_panel(click.event_target()) {
        click.propagate(false);
        return;
    }
    if selection_drag.suppresses_click(click.pointer.id, click.event_target())
        || value_drag.suppresses_click(click.pointer.id, click.event_target())
    {
        click.propagate(false);
        return;
    }
    let entity = click.event_target();
    if pointer
        .widgets
        .get(entity)
        .is_ok_and(|(_, _, _, disabled)| disabled)
    {
        click.propagate(false);
        return;
    }
    if pointer
        .place(
            entity,
            &click.hit,
            &click.pointer.location(),
            click.count,
            &mut commands,
        )
        .is_some()
    {
        click.propagate(false);
    } else if pointer.panels.contains(entity) {
        editor::classify_widget_click(entity, &editor, &mut blur);
    }
}

pub(super) fn press(
    press: On<PointerPress>,
    pointer: FieldPointer,
    editor: Res<ImeEditorState>,
    mut blur: ResMut<ImeBlurIntent>,
) {
    if press.button != PointerButton::Primary
        || press.event_target() != press.original_event_target()
        || editor.is_overlay_panel(press.event_target())
    {
        return;
    }
    let entity = press.event_target();
    let widget = pointer.widgets.get(entity).ok();
    let panel_entity = widget.map_or(entity, |(_, _, owner, _)| owner.panel());
    if let Ok((panel, computed, transform, _, _)) = pointer.panels.get(panel_entity) {
        let record = if let Some((widget, kind, _, disabled)) = widget {
            computed.field_records().iter().find(|record| {
                *kind == WidgetKind::EditableField
                    && !disabled
                    && !record.duplicate_id
                    && record.field_id == *widget.id()
            })
        } else {
            press
                .hit
                .position
                .and_then(|position| render::project_flat_panel_hit(position, panel, transform))
                .and_then(|local| computed.field_at_local_position(local))
        };
        if record.is_some_and(|record| {
            pointer.active.active_target()
                == Some(&activation::panel_field_target(
                    panel,
                    panel_entity,
                    record.field_id.clone(),
                ))
        }) {
            return;
        }
    }
    editor::classify_widget_click(panel_entity, &editor, &mut blur);
}

#[derive(Resource, Default)]
pub(super) struct FieldSelectionDrag {
    active: Option<SelectionDrag>,
}

impl FieldSelectionDrag {
    fn suppresses_click(&self, pointer: PointerId, entity: Entity) -> bool {
        self.active
            .as_ref()
            .is_some_and(|drag| drag.pointer == pointer && drag.entity == entity)
    }
}

struct SelectionDrag {
    pointer:       PointerId,
    entity:        Entity,
    camera:        Entity,
    target:        ImeTarget,
    render_target: NormalizedRenderTarget,
    anchor:        usize,
}

pub(super) fn start_drag(
    mut start: On<PointerDragStart>,
    mut pointer: FieldPointer,
    mut selection_drag: ResMut<FieldSelectionDrag>,
    mut commands: Commands,
) {
    if start.button != PointerButton::Primary || pointer.active.is_composing() {
        return;
    }
    let Some((target, anchor)) = pointer.place(
        start.event_target(),
        &start.hit,
        &start.pointer.location(),
        1,
        &mut commands,
    ) else {
        return;
    };
    selection_drag.active = Some(SelectionDrag {
        pointer: start.pointer.id,
        entity: start.event_target(),
        camera: start.hit.camera,
        target,
        render_target: start.pointer.target.clone(),
        anchor,
    });
    start.propagate(false);
}

pub(super) fn drag(
    mut drag: On<PointerDrag>,
    mut pointer: FieldPointer,
    selection_drag: Res<FieldSelectionDrag>,
    value_drag: Res<FieldValueDrag>,
    cameras: Query<(&Camera, &GlobalTransform, Option<&RenderTarget>)>,
    primary: Query<Entity, With<PrimaryWindow>>,
    mut commands: Commands,
) {
    if drag.button != PointerButton::Primary {
        return;
    }
    if value_drag.suppresses_click(drag.pointer.id, drag.event_target()) {
        drag.propagate(false);
        return;
    }
    let Some(capture) = selection_drag.active.as_ref().filter(|capture| {
        capture.pointer == drag.pointer.id
            && capture.entity == drag.event_target()
            && pointer.active.active_target() == Some(&capture.target)
    }) else {
        return;
    };
    if pointer.active.is_composing() {
        return;
    }
    let (ImeTarget::WorldPanelField { panel, field_id }
    | ImeTarget::ScreenPanelField { panel, field_id }) = &capture.target
    else {
        return;
    };
    let Ok((panel, computed, transform, _, _)) = pointer.panels.get(*panel) else {
        return;
    };
    let Ok((camera, camera_transform, render_target)) = cameras.get(capture.camera) else {
        return;
    };
    let Ok(local) = render::project_flat_panel_ray_hit(
        &CapturedCameraRay {
            camera,
            camera_transform,
            render_target,
            primary_window: primary.single().ok(),
            captured_target: &capture.render_target,
            viewport_position: drag.pointer.position,
        },
        panel,
        transform,
    ) else {
        return;
    };
    let Some(record) = computed
        .field_records()
        .iter()
        .find(|record| !record.duplicate_id && record.field_id == *field_id)
    else {
        return;
    };
    let Some(text) = pointer.active.active_text() else {
        return;
    };
    let focus = editor::text_hit_at_x(
        text,
        local.x,
        horizontal_scroll(computed, record),
        record,
        &pointer.measurer,
    )
    .insertion;
    if let Some(changed) = pointer.active.apply_edit_command(
        ImeEditCommand::SelectRange {
            anchor: capture.anchor,
            focus,
        },
        &pointer.blocker,
    ) {
        commands.trigger(changed);
    }
    drag.propagate(false);
}

pub(super) fn end_drag(end: On<PointerDragEnd>, mut selection_drag: ResMut<FieldSelectionDrag>) {
    if end.button == PointerButton::Primary
        && selection_drag
            .active
            .as_ref()
            .is_some_and(|capture| capture.pointer == end.pointer.id)
    {
        selection_drag.active = None;
    }
}

pub(super) fn cancel_drag(
    cancel: On<PointerCancel>,
    mut selection_drag: ResMut<FieldSelectionDrag>,
) {
    if selection_drag
        .active
        .as_ref()
        .is_some_and(|capture| capture.pointer == cancel.pointer.id)
    {
        selection_drag.active = None;
    }
}
