//! Constants for cascade resolution.

/// Maximum stack size for a single `CascadeProperty` value.
///
/// Cascaded values are cloned during propagation. This budget keeps cascade
/// attributes as cheap handles or small value wrappers, not owned render data.
pub(crate) const CASCADE_ATTRIBUTE_BYTES: usize = 32;
