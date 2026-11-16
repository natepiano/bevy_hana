//! Stable source identity and its acknowledged observation epoch.

use crate::RunOffset;

/// Stable ordinary source identity, equal to its diagnostic path string.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceKey(pub(super) String);

impl SourceKey {
    /// Returns the stable source identity.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

/// A continuous period of observed source evidence.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObservationEpoch(u64);

impl ObservationEpoch {
    pub(crate) const fn initial() -> Self { Self(0) }
    pub(crate) const fn next(self) -> Self { Self(self.0.saturating_add(1)) }

    /// Returns the run-local epoch generation.
    #[must_use]
    pub const fn generation(self) -> u64 { self.0 }
}

/// Source activation acknowledged at a service boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveMetricSource {
    /// Stable source path.
    pub source:       SourceKey,
    /// Continuous evidence epoch shared by concurrent consumers.
    pub epoch:        ObservationEpoch,
    /// Boundary when this epoch became effective.
    pub effective_at: RunOffset,
}
