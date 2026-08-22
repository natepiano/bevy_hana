mod context;
mod convex_hull;
mod edge;
mod frame;
mod screen_space;

#[cfg(test)]
pub(super) use context::FitOverlayCameraContext;
pub(super) use context::FitOverlayEmptyReason;
pub(super) use convex_hull::convex_hull_2d;
pub(super) use convex_hull::project_vertices_to_2d;
pub(super) use edge::Edge;
pub(super) use frame::FitOverlayFrame;
pub(super) use frame::FitOverlayLayout;
pub(super) use frame::resolve_fit_overlay_frame;
pub(super) use screen_space::MarginBalance;
pub(super) use screen_space::boundary_edge_center;
pub(super) use screen_space::horizontal_balance;
pub(super) use screen_space::margin_percentage;
pub(super) use screen_space::norm_to_viewport;
pub(super) use screen_space::normalized_to_world;
pub(super) use screen_space::screen_edge_center;
pub(super) use screen_space::vertical_balance;
