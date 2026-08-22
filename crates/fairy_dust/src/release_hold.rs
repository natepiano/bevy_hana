//! Small UI helper for labels that should remain visible briefly after input ends.

use std::time::Duration;

/// State returned by [`ReleaseHold::update`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldState<'a, T> {
    /// A fresh active value is present this frame.
    Active(&'a T),
    /// No fresh value is present, but the last value is still within the hold window.
    Held(&'a T),
    /// The hold window has elapsed and the caller should render its idle state.
    Idle,
}

/// Remembers the last active value for a fixed duration after input ends.
#[derive(Clone, Debug)]
pub struct ReleaseHold<T> {
    hold:      Duration,
    remaining: Duration,
    value:     Option<T>,
}

impl<T> ReleaseHold<T> {
    /// Creates a hold buffer with the given release duration.
    #[must_use]
    pub const fn new(hold: Duration) -> Self {
        Self {
            hold,
            remaining: Duration::ZERO,
            value: None,
        }
    }

    /// Clears any remembered value and returns to idle immediately.
    pub fn clear(&mut self) {
        self.remaining = Duration::ZERO;
        self.value = None;
    }

    /// Updates the hold buffer and returns the value the caller should render.
    pub fn update(&mut self, delta: Duration, active: Option<T>) -> HoldState<'_, T> {
        if let Some(active) = active {
            self.remaining = self.hold;
            return HoldState::Active(self.value.insert(active));
        }

        self.remaining = self.remaining.saturating_sub(delta);
        if self.remaining == Duration::ZERO {
            self.value = None;
            return HoldState::Idle;
        }

        match self.value.as_ref() {
            Some(value) => HoldState::Held(value),
            None => HoldState::Idle,
        }
    }
}

impl<T> Default for ReleaseHold<T> {
    fn default() -> Self { Self::new(crate::constants::CUBE_FACE_PANEL_RELEASE_HOLD) }
}
