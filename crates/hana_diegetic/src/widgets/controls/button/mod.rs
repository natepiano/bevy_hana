mod callback;
mod events;
mod pop_up;
mod presentation;
mod press;

use bevy::prelude::In;
use bevy::prelude::IntoSystem;
pub(in crate::widgets) use callback::ButtonCallback;
pub(in crate::widgets) use callback::ButtonCallbackHandle;
pub(in crate::widgets) use callback::ButtonCallbackPolicy;
pub(in crate::widgets) use callback::dispatch_click_callback;
pub use events::ButtonActivationSource;
pub use events::ButtonCancelCause;
pub use events::ButtonCanceled;
pub use events::ButtonClicked;
pub use events::ButtonPressSource;
pub use events::ButtonPressed;
pub use events::ButtonReleased;
pub(in crate::widgets) use presentation::present_button_state;
pub(crate) use press::ButtonCaptures;
pub use press::ButtonLatch;
pub(crate) use press::ButtonPress;
pub(in crate::widgets) use press::apply_raw_release;
pub(in crate::widgets) use press::cancel_before_widget_despawn;
pub(in crate::widgets) use press::cancel_button_press;
pub(in crate::widgets) use press::cancel_from_disabled;
pub(in crate::widgets) use press::cancel_from_drag_end;
pub(in crate::widgets) use press::cancel_from_focus_loss;
pub(in crate::widgets) use press::cancel_from_pointer;
pub(in crate::widgets) use press::cancel_from_pointer_removal;
pub(in crate::widgets) use press::cancel_from_widget_removal;
pub(in crate::widgets) use press::capture_reconciled_button_press;
pub(in crate::widgets) use press::click_from_pointer;
pub(crate) use press::finalize_panel_buttons;
pub(in crate::widgets) use press::handle_semantic_intent;
pub(in crate::widgets) use press::pointer_out_of_button;
pub(in crate::widgets) use press::pointer_over_button;
pub(in crate::widgets) use press::press_from_pointer;
pub use press::release_button;
pub(in crate::widgets) use press::release_from_pointer;

/// Authored behavior for a panel button.
///
/// Declare one inline with [`El::button`](crate::El::button), or build one here
/// and attach it with [`El::widget`](crate::El::widget). A button carries
/// behavior: its resting look stays on the element's ordinary
/// [`El::background`](crate::El::background),
/// [`El::border`](crate::El::border), and
/// [`El::material`](crate::El::material) declarations, and its per-state look
/// on the element's state builders. The [`Button::pop_up`] presentation also
/// confines its paint to the layout bounds reserved for that control.
#[must_use]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Button {
    mode:         ButtonMode,
    callback:     ButtonCallbackPolicy,
    paint_bounds: ButtonPaintBounds,
}

/// Whether authored antialiasing may extend beyond the layout rectangle.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ButtonPaintBounds {
    #[default]
    Antialiased,
    InsideLayout,
}

/// Whether a button returns to rest after activation or retains its state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ButtonMode {
    /// Each activation is momentary.
    #[default]
    Momentary,
    /// Each activation alternates between pressed and released.
    Toggle,
}

impl Button {
    /// Creates a button declaration with default behavior.
    pub const fn new() -> Self {
        Self {
            mode:         ButtonMode::Momentary,
            callback:     ButtonCallbackPolicy::NotifyObserversOnly,
            paint_bounds: ButtonPaintBounds::Antialiased,
        }
    }

    pub(crate) const fn paints_inside_layout_bounds(&self) -> bool {
        matches!(self.paint_bounds, ButtonPaintBounds::InsideLayout)
    }

    /// Retains the pressed look until the next activation or an explicit release.
    pub const fn toggle(mut self) -> Self {
        self.mode = ButtonMode::Toggle;
        self
    }

    /// Returns the button's activation mode.
    #[must_use]
    pub const fn mode(&self) -> ButtonMode { self.mode }

    /// Runs `system` with each completed [`ButtonClicked`] for this button.
    ///
    /// The callback is an ordinary Bevy system taking `In<ButtonClicked>`
    /// plus any other system parameters. Reify registers it once as a tracked
    /// system; authoring the same widget id with a different
    /// callback releases the prior tracked handle, and dropping the final
    /// handle unregisters the system. This sugar complements direct
    /// [`ButtonClicked`] observation — a global or entity-scoped observer
    /// reads the widget entity from the event target.
    pub fn on_click<M>(mut self, system: impl IntoSystem<In<ButtonClicked>, (), M>) -> Self {
        self.set_callback(system);
        self
    }

    pub(crate) fn set_callback<M>(&mut self, system: impl IntoSystem<In<ButtonClicked>, (), M>) {
        self.callback = ButtonCallbackPolicy::AlsoRun(ButtonCallback::new(system));
    }

    pub(in crate::widgets) const fn callback(&self) -> &ButtonCallbackPolicy { &self.callback }
}
