//! Runtime retention budgets supplied by the application's configuration.

use std::num::NonZeroUsize;
use std::time::Duration;

use bevy_ecs::prelude::Resource;
use serde::Deserialize;
use serde::Serialize;

/// Service budgets, inserted before [`crate::MetricsPlugin`] or edited before subscribing.
///
/// Applications can deserialize these settings through their existing configuration loader.
/// The service performs no independent file or environment lookup. Changes affect new
/// subscriptions; existing subscriptions keep their acknowledged retention.
#[derive(Clone, Debug, Resource, Serialize, Deserialize)]
pub struct MetricsSettings {
    /// Maximum retained observations per path, including a counter baseline slot.
    pub max_retained_samples: NonZeroUsize,
    /// Longest trailing interval a new subscription may request.
    pub max_trailing:         Duration,
}

impl Default for MetricsSettings {
    fn default() -> Self {
        Self {
            max_retained_samples: NonZeroUsize::new(8192).unwrap_or(NonZeroUsize::MIN),
            max_trailing:         Duration::from_secs(60),
        }
    }
}
