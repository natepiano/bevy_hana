//! Tracks primary presses that begin on a widget until their pointer releases.

use std::collections::HashSet;

use bevy::picking::events::PointerPress;
use bevy::picking::pointer::PointerButton;
use bevy::picking::pointer::PointerId;
use bevy::prelude::ButtonInput;
use bevy::prelude::MouseButton;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Touches;

use crate::widgets::WidgetOf;

/// Whether a held primary pointer press began on a widget.
///
/// Disabled widgets still own their surface. Ownership lasts until release,
/// even if the pointer leaves the widget or the widget cancels its own action.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WidgetPrimaryPressOwnership {
    /// No widget owns a held primary press.
    #[default]
    Unclaimed,
    /// At least one held primary press began on a widget.
    HeldByWidget,
}

/// The pointer identities behind the published ownership state.
#[derive(Resource, Default)]
pub(in crate::widgets) struct WidgetPrimaryPresses(HashSet<PointerId>);

/// Observe the original target, including presses stopped by widget observers.
pub(in crate::widgets) fn record_widget_primary_press(
    press: On<PointerPress>,
    widgets: Query<&WidgetOf>,
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    touches: Option<Res<Touches>>,
    mut presses: ResMut<WidgetPrimaryPresses>,
    mut ownership: ResMut<WidgetPrimaryPressOwnership>,
) {
    if press.button != PointerButton::Primary
        || !widgets.contains(press.original_event_target())
        || !primary_pointer_is_held(press.pointer.id, buttons.as_deref(), touches.as_deref())
    {
        return;
    }
    match press.pointer.id {
        PointerId::Mouse | PointerId::Touch(_) => {
            presses.0.insert(press.pointer.id);
            *ownership = WidgetPrimaryPressOwnership::HeldByWidget;
        },
        PointerId::Custom(_) => {},
    }
}

/// Retire released pointers before picking emits the next frame's presses.
pub(in crate::widgets) fn release_widget_primary_presses(
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    touches: Option<Res<Touches>>,
    mut presses: ResMut<WidgetPrimaryPresses>,
    mut ownership: ResMut<WidgetPrimaryPressOwnership>,
) {
    presses.0.retain(|pointer| {
        primary_pointer_is_held(*pointer, buttons.as_deref(), touches.as_deref())
    });
    let next = if presses.0.is_empty() {
        WidgetPrimaryPressOwnership::Unclaimed
    } else {
        WidgetPrimaryPressOwnership::HeldByWidget
    };
    if *ownership != next {
        *ownership = next;
    }
}

fn primary_pointer_is_held(
    pointer: PointerId,
    buttons: Option<&ButtonInput<MouseButton>>,
    touches: Option<&Touches>,
) -> bool {
    match pointer {
        PointerId::Mouse => buttons.is_some_and(|buttons| buttons.pressed(MouseButton::Left)),
        PointerId::Touch(id) => touches.is_some_and(|touches| touches.get_pressed(id).is_some()),
        PointerId::Custom(_) => false,
    }
}
