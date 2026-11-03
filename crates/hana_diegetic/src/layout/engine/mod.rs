//! Core layout computation engine.
//!
//! Implements a Clay-inspired two-pass layout algorithm:
//! 1. **Sizing pass** — BFS traversal determines element dimensions (called twice: X then Y).
//! 2. **Positioning pass** — DFS traversal computes final positions and emits render commands.
//!
//! The engine is fully self-contained with no global state. Multiple instances can run
//! concurrently on different threads without interference.

mod layout_engine;
mod positioning;
mod sizing;
mod wrapping;

pub(crate) use layout_engine::CapHeightMeasurement;
pub(crate) use layout_engine::CapHeightMeasurer;
pub(crate) use layout_engine::CapHeightMetrics;
pub use layout_engine::LayoutEngine;
pub use layout_engine::LayoutResult;
pub(crate) use layout_engine::MeasureCapHeightFn;
pub use layout_engine::MeasureTextFn;
pub(crate) use layout_engine::RichMeasurement;
pub(crate) use layout_engine::ScrollSolveInput;
pub(crate) use layout_engine::ScrollViewSolve;
pub use layout_engine::SolvedScroll;
pub use layout_engine::SolvedScrollIndicator;
pub use layout_engine::VisibleScrollRegion;
pub(crate) use sizing::content_box_bounds;

#[cfg(test)]
mod clay_parity;

#[cfg(test)]
mod integration_tests;
