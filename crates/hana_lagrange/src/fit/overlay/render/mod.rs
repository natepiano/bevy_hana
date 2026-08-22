mod fit_target_bounds;
mod labels;
mod lines;
mod reconciliation;
mod visual;

pub use fit_target_bounds::FitTargetOverlayConfig;
pub(super) use fit_target_bounds::draw_fit_target_bounds;
pub(super) use fit_target_bounds::on_remove_fit_visualization;
pub(super) use lines::FitOverlayLineMaterial;
pub(super) use lines::FitOverlayLineMaterials;
pub(super) use reconciliation::cleanup_orphan_fit_overlay_visuals;
pub(super) use reconciliation::deduplicate_fit_overlay_visuals;
