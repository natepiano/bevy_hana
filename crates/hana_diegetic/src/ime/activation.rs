//! Panel picking activation for authored editable fields.

use bevy::camera::NormalizedRenderTarget;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::diagnostic::FrameCount;
use bevy::ecs::entity::ContainsEntity;
use bevy::prelude::Add;
use bevy::prelude::Camera;
use bevy::prelude::Click;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::GlobalTransform;
use bevy::prelude::On;
use bevy::prelude::Pointer;
use bevy::prelude::PointerButton;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::window::PrimaryWindow;

use super::ActiveImeSession;
use super::ImeOpenSession;
use super::ImeTarget;
use super::editor::PendingImePanelAnchor;
use crate::ComputedDiegeticPanel;
use crate::DiegeticPanel;
use crate::render;
use crate::widgets;
use crate::widgets::PanelWidget;
use crate::widgets::SemanticWidgetIntent;
use crate::widgets::WidgetFocusAuthority;
use crate::widgets::WidgetKind;
use crate::widgets::WidgetOf;

pub(super) fn observe_panel_clicks(trigger: On<Add, DiegeticPanel>, mut commands: Commands) {
    commands
        .entity(trigger.event_target())
        .observe(open_from_panel_click);
}

fn open_from_panel_click(
    mut click: On<Pointer<Click>>,
    panels: Query<(&DiegeticPanel, &ComputedDiegeticPanel, &GlobalTransform)>,
    active_session: Res<ActiveImeSession>,
    frame_count: Option<Res<FrameCount>>,
    mut pending_anchor: ResMut<PendingImePanelAnchor>,
    mut commands: Commands,
) {
    if click.button != PointerButton::Primary || click.count < 2 {
        return;
    }

    let panel_entity = click.event_target();
    let Ok((panel, computed, transform)) = panels.get(panel_entity) else {
        return;
    };
    let Some(panel_local) = click
        .hit
        .position
        .and_then(|position| render::project_flat_panel_hit(position, panel, transform))
    else {
        return;
    };
    let Some(record) = computed.field_at_local_position(panel_local) else {
        return;
    };
    let Some(window) = pointer_window(&click) else {
        return;
    };

    let target = if panel.coordinate_space().is_screen() {
        ImeTarget::ScreenPanelField {
            panel:    panel_entity,
            field_id: record.field_id.clone(),
        }
    } else {
        ImeTarget::WorldPanelField {
            panel:    panel_entity,
            field_id: record.field_id.clone(),
        }
    };
    if active_session.active_target() == Some(&target) {
        return;
    }

    click.propagate(false);
    pending_anchor.store(
        panel_entity,
        record.field_id.clone(),
        Some(click.hit.camera),
        window,
        record.bounds,
        record.presentation().clone(),
    );
    commands.trigger(ImeOpenSession {
        window,
        target,
        initial_text: record.display_text.clone(),
        field_spec: record.field_spec.clone(),
        anchor: None,
    });

    if let Some(frame_count) = frame_count {
        bevy::log::trace!(
            target: "hana_diegetic::ime",
            "captured editable field activation on frame {}",
            frame_count.0
        );
    }
}

pub(super) fn open_from_semantic_activation(
    intent: On<SemanticWidgetIntent>,
    widgets: Query<(&PanelWidget, &WidgetKind, &WidgetOf)>,
    panels: Query<(
        &DiegeticPanel,
        &ComputedDiegeticPanel,
        Option<&RenderLayers>,
    )>,
    cameras: Query<(
        Entity,
        &Camera,
        &GlobalTransform,
        Option<&RenderTarget>,
        Option<&RenderLayers>,
    )>,
    windows: Query<(), With<Window>>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    focus_authority: Res<WidgetFocusAuthority>,
    mut pending_anchor: ResMut<PendingImePanelAnchor>,
    mut commands: Commands,
) {
    let SemanticWidgetIntent::Activate { entity, window } = *intent.event() else {
        return;
    };
    let Ok((widget, kind, widget_of)) = widgets.get(entity) else {
        return;
    };
    if *kind != WidgetKind::EditableField {
        return;
    }
    let panel_entity = widget_of.panel();
    let Ok((panel, computed, panel_layers)) = panels.get(panel_entity) else {
        return;
    };
    let Some(record) = computed
        .field_records()
        .iter()
        .find(|record| !record.duplicate_id && record.field_id == *widget.id())
    else {
        return;
    };
    let camera = if panel.coordinate_space().is_screen() {
        None
    } else {
        let target_layers = panel_layers
            .cloned()
            .unwrap_or_else(|| RenderLayers::layer(0));
        let preferred = focus_authority.interaction_camera(window, entity);
        widgets::select_window_presentation_camera(
            preferred,
            window,
            &target_layers,
            &cameras,
            &windows,
            &primary_window,
        )
    };
    if !panel.coordinate_space().is_screen() && camera.is_none() {
        return;
    }

    pending_anchor.store(
        panel_entity,
        record.field_id.clone(),
        camera,
        window,
        record.bounds,
        record.presentation().clone(),
    );
    commands.trigger(ImeOpenSession {
        window,
        target: panel_field_target(panel, panel_entity, record.field_id.clone()),
        initial_text: record.display_text.clone(),
        field_spec: record.field_spec.clone(),
        anchor: None,
    });
}

const fn panel_field_target(
    panel: &DiegeticPanel,
    panel_entity: Entity,
    field_id: crate::PanelElementId,
) -> ImeTarget {
    if panel.coordinate_space().is_screen() {
        ImeTarget::ScreenPanelField {
            panel: panel_entity,
            field_id,
        }
    } else {
        ImeTarget::WorldPanelField {
            panel: panel_entity,
            field_id,
        }
    }
}

fn pointer_window(click: &Pointer<Click>) -> Option<Entity> {
    match &click.pointer_location.target {
        NormalizedRenderTarget::Window(window_ref) => Some(window_ref.entity()),
        NormalizedRenderTarget::Image(_)
        | NormalizedRenderTarget::TextureView(_)
        | NormalizedRenderTarget::None { .. } => None,
    }
}
