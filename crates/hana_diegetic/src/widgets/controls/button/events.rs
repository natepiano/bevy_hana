use bevy::picking::pointer::PointerId;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;

use crate::PanelElementId;

/// Input path holding a button press.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonPressSource {
    /// A pointer holds the button.
    Pointer(PointerId),
    /// A semantic action holds the button.
    Semantic,
}

/// Reports the beginning of a button press.
#[derive(Clone, Debug, EntityEvent)]
pub struct ButtonPressed {
    /// Live button entity receiving the press.
    #[event_target]
    pub entity: Entity,
    /// Panel-local button id.
    pub id:     PanelElementId,
    /// Input path that began the press.
    pub source: ButtonPressSource,
}

/// Reports the valid release of a button press.
#[derive(Clone, Debug, EntityEvent)]
pub struct ButtonReleased {
    /// Live button entity receiving the release.
    #[event_target]
    pub entity: Entity,
    /// Panel-local button id.
    pub id:     PanelElementId,
    /// Input path that completed the press.
    pub source: ButtonPressSource,
}

/// Reports pointer or semantic activation of a button.
#[derive(Clone, Debug, EntityEvent)]
pub struct ButtonClicked {
    /// Live button entity receiving activation.
    #[event_target]
    pub entity: Entity,
    /// Panel-local button id.
    pub id:     PanelElementId,
    /// Path that activated the button.
    pub source: ButtonActivationSource,
}

/// Input path that completed a button activation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonActivationSource {
    /// A pointer completed its press and click.
    Pointer(PointerId),
    /// A semantic action activated the button.
    Semantic,
}

impl From<ButtonPressSource> for ButtonActivationSource {
    fn from(source: ButtonPressSource) -> Self {
        match source {
            ButtonPressSource::Pointer(pointer_id) => Self::Pointer(pointer_id),
            ButtonPressSource::Semantic => Self::Semantic,
        }
    }
}

/// Reports a button press that did not reach release.
#[derive(Clone, Debug, EntityEvent)]
pub struct ButtonCanceled {
    /// Live button entity whose press was canceled.
    #[event_target]
    pub entity: Entity,
    /// Panel-local button id.
    pub id:     PanelElementId,
    /// Input path whose press was canceled.
    pub source: ButtonPressSource,
    /// Reason the press ended without release.
    pub cause:  ButtonCancelCause,
}

/// Reason a button press was canceled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonCancelCause {
    /// The pointer input stream reported cancellation.
    PointerCanceled,
    /// The captured pointer entity or its [`PointerId`] was removed.
    PointerRemoved,
    /// The captured pointer released without a valid button release.
    CaptureLost,
    /// The button became disabled while pressed.
    Disabled,
    /// The widget was removed or its owning panel role ended.
    WidgetRemoved,
    /// The same widget id changed to another widget kind.
    WidgetKindChanged,
    /// Semantic input explicitly canceled the press.
    Explicit,
    /// Focus moved away from the button or its window lost focus.
    FocusLost,
    /// The semantic input hold ended abnormally.
    InputInterrupted,
}
