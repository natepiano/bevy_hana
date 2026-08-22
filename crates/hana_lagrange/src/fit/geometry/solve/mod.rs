//! Fit algorithm for framing objects in the camera view.
//!
//! Provides screen-space projection, margin calculation, and a binary search convergence
//! loop that finds the optimal camera radius and focus to frame a set of mesh vertices
//! with a specified margin.

mod fit_solution;
mod focus;
mod margins;
mod radius_search;

pub use fit_solution::FitError;
pub use fit_solution::FitSolution;
pub use radius_search::calculate_fit;
