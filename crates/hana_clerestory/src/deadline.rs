use std::time::Duration;

use bevy::prelude::Reflect;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;

/// A hard bound on work the operating system is doing on its own.
///
/// Advancing the clock and reading the result are one call, so the answer cannot be obtained
/// without time having moved.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub(crate) struct OperatingSystemWorkDeadline {
    remaining: Timer,
}

impl OperatingSystemWorkDeadline {
    pub(crate) fn new(seconds: f32) -> Self {
        Self {
            remaining: Timer::from_seconds(seconds, TimerMode::Once),
        }
    }

    #[must_use]
    pub(crate) fn advance(&mut self, delta: Duration) -> OperatingSystemWorkDeadlineStatus {
        self.remaining.tick(delta);
        if self.remaining.is_finished() {
            OperatingSystemWorkDeadlineStatus::Expired
        } else {
            OperatingSystemWorkDeadlineStatus::Pending
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperatingSystemWorkDeadlineStatus {
    /// Time remains.
    Pending,
    /// The deadline passed. Every later call answers the same.
    Expired,
}

/// Continuous time for which one window observation has remained unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub(crate) struct WindowStabilityInterval {
    elapsed: Timer,
}

impl WindowStabilityInterval {
    pub(crate) fn new(seconds: f32) -> Self {
        Self {
            elapsed: Timer::from_seconds(seconds, TimerMode::Once),
        }
    }

    #[must_use]
    pub(crate) fn advance(&mut self, delta: Duration) -> WindowStabilityIntervalStatus {
        self.elapsed.tick(delta);
        if self.elapsed.is_finished() {
            WindowStabilityIntervalStatus::Stable
        } else {
            WindowStabilityIntervalStatus::Changing
        }
    }

    pub(crate) fn reset(&mut self) { self.elapsed.reset(); }

    #[must_use]
    pub(crate) fn elapsed_secs(&self) -> f32 { self.elapsed.elapsed_secs() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowStabilityIntervalStatus {
    /// The observation has not remained unchanged for the full interval.
    Changing,
    /// The observation has remained unchanged for the full interval.
    Stable,
}
