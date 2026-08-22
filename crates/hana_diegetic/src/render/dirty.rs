//! Two-state flag shared by every batch path: whether a tracked value (a
//! record/instance buffer, a batch's bounds, or a path atlas) still matches
//! the live batch data or must be recomputed and re-uploaded.

/// Whether a tracked batch value needs recomputation or re-upload.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Dirty {
    /// The tracked value matches the current batch data.
    #[default]
    No,
    /// The tracked value must be recomputed or re-uploaded.
    Yes,
}

impl Dirty {
    /// Flags the value as needing recomputation or re-upload.
    pub(crate) const fn mark(&mut self) { *self = Self::Yes; }

    /// Flags the value as matching the current batch data.
    pub(crate) const fn clear(&mut self) { *self = Self::No; }

    /// Whether the value needs recomputation or re-upload.
    #[must_use]
    pub(crate) const fn is_set(self) -> bool { matches!(self, Self::Yes) }
}
