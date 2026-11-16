//! Bounded history capacity, separate from consumer and source identities.

pub(super) const LATEST_ONLY_CAPACITY: usize = 1;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct RetentionCapacity(usize);

impl RetentionCapacity {
    pub(crate) const fn new(samples: usize) -> Self { Self(samples) }
    pub(crate) const fn latest() -> Self { Self(LATEST_ONLY_CAPACITY) }
    pub(crate) const fn get(self) -> usize { self.0 }
}
