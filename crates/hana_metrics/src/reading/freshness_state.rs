use std::time::Duration;

use crate::Freshness;
use crate::RunOffset;

pub(super) enum FreshnessState {
    Current,
    Stale(Duration),
}
impl FreshnessState {
    pub(super) fn at(policy: Freshness, measured_at: RunOffset, now: RunOffset) -> Self {
        let age = now.duration().saturating_sub(measured_at.duration());
        match policy {
            Freshness::LiveWithin(limit) if age > limit => Self::Stale(age),
            _ => Self::Current,
        }
    }
}
