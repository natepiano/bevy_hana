//! Declared smoothing state updated from the accepted observation stream.

use std::time::Duration;

use crate::RunOffset;
use crate::SmoothingMethod;

pub(crate) enum FilterState {
    Unseeded,
    Seeded { value: f64, at: RunOffset },
}

impl FilterState {
    pub(super) fn observe(&mut self, sample: f64, at: RunOffset, method: SmoothingMethod) {
        let (alpha, reset_after) = match method {
            SmoothingMethod::Disabled => return,
            SmoothingMethod::FixedAlpha { alpha, reset_after } => (alpha, reset_after),
            SmoothingMethod::TimeBased {
                time_constant,
                reset_after,
            } => {
                let elapsed = match *self {
                    Self::Unseeded => Duration::ZERO,
                    Self::Seeded { at: previous, .. } => {
                        at.duration().saturating_sub(previous.duration())
                    },
                };
                (
                    -(-elapsed.as_secs_f64() / time_constant.as_secs_f64()).exp_m1(),
                    reset_after,
                )
            },
        };
        let value = match *self {
            Self::Seeded {
                value,
                at: previous,
            } if at.duration().saturating_sub(previous.duration()) <= reset_after => {
                sample.mul_add(alpha, value * (1.0 - alpha))
            },
            Self::Unseeded | Self::Seeded { .. } => sample,
        };
        *self = Self::Seeded { value, at };
    }
}
