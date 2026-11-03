mod capture;
mod dispatch;
mod drag_claim;
mod picking;
mod scroll;
mod size_gate;

#[cfg(test)]
pub(in crate::widgets) use capture::CapturedWidget;
pub(in crate::widgets) use capture::WidgetCaptures;
pub(in crate::widgets) use capture::trigger_immediate;
pub(in crate::widgets) use dispatch::reconcile_pointer_input;
pub use drag_claim::WidgetPrimaryPressOwnership;
pub(in crate::widgets) use drag_claim::WidgetPrimaryPresses;
pub(in crate::widgets) use drag_claim::record_widget_primary_press;
pub(in crate::widgets) use drag_claim::release_widget_primary_presses;
pub use picking::FacePicking;
pub use picking::PanelPicking;
pub(in crate::widgets) use picking::update_hits;
pub use scroll::HoveredScrollTargets;
pub use scroll::PanelScrolled;
pub use scroll::PointerScrollTarget;
pub(in crate::widgets) use scroll::scroll_from_pointer;
pub use size_gate::SizeGateExempt;
pub use size_gate::WidgetScreenSizeGate;
