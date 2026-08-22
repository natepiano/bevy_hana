mod anchor;
mod projection;
mod solve;

pub use anchor::FitAnchor;
#[cfg(feature = "fit_overlay")]
pub(super) use projection::ProjectionBasis;
#[cfg(feature = "fit_overlay")]
pub(super) use projection::ProjectionMode;
#[cfg(feature = "fit_overlay")]
pub(super) use projection::ScreenSpaceBounds;
pub(super) use projection::extract_mesh_vertices;
#[cfg(feature = "fit_overlay")]
pub(super) use projection::project_point;
#[cfg(feature = "fit_overlay")]
pub(super) use projection::projection_aspect_ratio;
pub(super) use solve::FitError;
pub(super) use solve::FitSolution;
pub(super) use solve::calculate_fit;
