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

pub use layout_engine::LayoutEngine;
pub use layout_engine::LayoutResult;
pub use layout_engine::MeasureTextFn;
pub(crate) use sizing::content_box_bounds;

#[cfg(test)]
mod clay_parity;

#[cfg(test)]
mod integration_tests;
