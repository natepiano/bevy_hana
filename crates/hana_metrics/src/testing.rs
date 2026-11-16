//! Deterministic run clock for metrics tests.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use bevy_ecs::prelude::Resource;

use crate::recording;

/// Run clock advanced explicitly by a test rather than by elapsed wall time.
/// Insert before adding [`crate::MetricsPlugin`].
#[derive(Clone, Default, Resource)]
pub struct ManualRunClock {
    pub(crate) elapsed: Arc<Mutex<Duration>>,
}

impl ManualRunClock {
    /// Creates a run clock at zero.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Moves the clock forward by the supplied duration.
    pub fn advance(&self, elapsed: Duration) {
        let mut current = recording::lock(&self.elapsed);
        *current = current.saturating_add(elapsed);
    }
}
