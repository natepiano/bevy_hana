//! Retained widget visual slots and state-only presentation overrides.
//!
//! A visual slot is a stable private id authored on an ordinary layout
//! element inside a widget subtree. Layout output records each slot's
//! element index on the owning `ComputedWidgetRecord`; reify copies those
//! references onto the widget entity as [`WidgetVisualSlots`]. Widget state
//! writes [`WidgetVisualOverrides`], and [`dispatch_visual_overrides`]
//! resolves the changed slot set into the [`VisualOverrideIndex`] the four
//! retained-batch routes read: `route_sdf_batch_records`,
//! `route_image_batch_records`, `update_panel_text_batches`, and
//! `reconcile_panel_line_batches`.
//!
//! An override never rewrites `DiegeticPanel`, regenerates the `LayoutTree`,
//! changes `ComputedDiegeticPanel`, or runs geometry solving: it is applied
//! while the routes rebuild retained batch records, so authored data stays
//! untouched and clearing the override restores the authored appearance.
//! Overrides patch records that layout already emitted; they never create a
//! record for an unauthored fill, border, image, text, or panel-line role.

mod index;
mod overrides;
mod resolve;
mod slots;

pub(crate) use index::VisualOverrideIndex;
pub(in crate::widgets) use index::dispatch_visual_overrides;
pub(crate) use overrides::VisualSlotOverride;
pub(crate) use overrides::WidgetVisualOverrides;
pub(in crate::widgets) use overrides::dirty_widgets;
pub(in crate::widgets) use overrides::layout_delta_to_render_offset;
pub(in crate::widgets) use overrides::write_widget_overrides;
pub(in crate::widgets) use resolve::resolve_part_overrides;
pub(crate) use slots::ComputedVisualSlot;
pub(crate) use slots::VisualSlotId;
pub(crate) use slots::WidgetVisualSlots;
