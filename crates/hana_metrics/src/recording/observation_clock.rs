//! Monotonic or explicitly advanced time within one metrics run.

use std::time::Instant;

use bevy_ecs::prelude::Resource;

#[cfg(feature = "test")]
use super::lock;
#[cfg(feature = "test")]
use crate::ManualRunClock;
use crate::RunOffset;

/// Run-local clock passed to interval-summary producers.
#[derive(Clone, Resource)]
pub struct ObservationClock {
    pub(crate) origin: Instant,
    pub(super) source: ClockSource,
}

#[derive(Clone)]
pub(super) enum ClockSource {
    Monotonic,
    #[cfg(feature = "test")]
    Manual(ManualRunClock),
}

impl ObservationClock {
    pub(super) fn monotonic() -> Self {
        Self {
            origin: Instant::now(),
            source: ClockSource::Monotonic,
        }
    }

    /// Returns the current offset from this run's origin.
    #[must_use]
    pub fn now(&self) -> RunOffset {
        RunOffset::new(match &self.source {
            ClockSource::Monotonic => self.origin.elapsed(),
            #[cfg(feature = "test")]
            ClockSource::Manual(clock) => *lock(&clock.elapsed),
        })
    }
}
