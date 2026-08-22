//! Cap primitives for panel lines.
//!
//! Cap geometry resolved here is consumed by `layout::line` and emitted as
//! panel-shape primitives.

mod caps;

pub use caps::ArrowStyle;
pub use caps::CalloutCap;
pub(crate) use caps::CalloutCapPrimitiveKind;
pub(crate) use caps::ResolvedCalloutCap;
pub(crate) use caps::ResolvedCalloutCapPrimitive;
