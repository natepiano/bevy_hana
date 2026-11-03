mod id;
mod relation;
mod spec;

pub use id::PanelWidget;
pub(crate) use id::PanelWidgetIndex;
pub use id::PanelWidgetReader;
pub use relation::PanelWidgets;
pub use relation::WidgetOf;
pub(crate) use spec::ComputedWidgetRecord;
pub(crate) use spec::VisualElementCapabilities;
pub(crate) use spec::WidgetHitArea;
pub use spec::WidgetKind;
pub(crate) use spec::WidgetSpec;
pub(crate) use spec::validate_tree;
