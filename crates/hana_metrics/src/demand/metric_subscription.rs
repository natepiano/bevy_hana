//! Consumer subscriptions, selection validation and release on drop.

use std::collections::HashMap;
use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::{self};
use std::sync::Arc;
use std::sync::Mutex;

use bevy_diagnostic::DiagnosticPath;
use bevy_diagnostic::DiagnosticsStore;
use bevy_ecs::prelude::World;

use super::ActiveMetricSource;
use super::ActiveSubscription;
use super::DemandChanges;
use super::DemandLedger;
use super::LATEST_ONLY_CAPACITY;
use super::RetentionCapacity;
use super::SourceKey;
use super::SubscriptionBoundaries;
use super::SubscriptionId;
use crate::MeasurementFailure;
use crate::MetricCapabilityFailure;
use crate::MetricSelection;
use crate::MetricStatistic;
use crate::SampleWindow;
use crate::WindowReduction;
use crate::definition;
use crate::recording;
use crate::recording::MetricRegistry;

/// Subscription activation state, shared with the service controller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionStatus {
    /// Demand is queued and producers remain disabled until reconciliation.
    PendingActivation,
    /// All requested sources have acknowledged activation.
    Active {
        /// Effective source identities and observation epochs.
        sources: Vec<ActiveMetricSource>,
    },
    /// The consumer has released this subscription.
    Ended,
    /// The service cannot provide the requested evidence.
    Failed(MeasurementFailure),
}

/// Why a subscription could not acquire demand.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetricSubscriptionError {
    /// The shared metrics plugin has not been installed.
    ServiceNotInstalled,
    /// No descriptor is registered for a requested path.
    UnknownPath,
    /// The requested statistic is incompatible with the source declaration.
    Capability(MetricCapabilityFailure),
    /// A requested sample capacity or trailing duration exceeds service limits.
    RetentionExceeded,
}

impl Display for MetricSubscriptionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ServiceNotInstalled => formatter.write_str("metrics service is not installed"),
            Self::UnknownPath => formatter.write_str("unknown metric path"),
            Self::Capability(failure) => Display::fmt(failure, formatter),
            Self::RetentionExceeded => {
                formatter.write_str("requested retention exceeds service limits")
            },
        }
    }
}
impl Error for MetricSubscriptionError {}

/// Consumer-owned demand that is released on drop at the next first boundary.
pub struct MetricSubscription {
    id:                    SubscriptionId,
    ledger:                Arc<DemandChanges>,
    status:                Arc<Mutex<SubscriptionStatus>>,
    pub(crate) boundaries: SubscriptionBoundaries,
    selections:            Vec<MetricSelection>,
}

impl MetricSubscription {
    /// Returns activation acknowledgements without changing collection demand.
    #[must_use]
    pub fn status(&self) -> SubscriptionStatus {
        let status = recording::lock(&self.status).clone();
        if matches!(status, SubscriptionStatus::Active { .. }) {
            let sources = &self.boundaries;
            let sources = recording::lock(sources)
                .iter()
                .map(|(path, boundary)| {
                    let activation = recording::lock(&boundary.activation);
                    ActiveMetricSource {
                        source:       SourceKey(path.as_str().to_owned()),
                        epoch:        activation.epoch,
                        effective_at: if activation.epoch == boundary.epoch {
                            boundary.effective_at
                        } else {
                            activation.effective_at
                        },
                    }
                })
                .collect();
            SubscriptionStatus::Active { sources }
        } else {
            status
        }
    }
    pub(crate) fn is_active(&self) -> bool {
        matches!(
            *recording::lock(&self.status),
            SubscriptionStatus::Active { .. }
        )
    }

    pub(crate) fn holds(&self, selection: &MetricSelection) -> bool {
        self.selections.contains(selection)
    }

    pub(crate) fn holds_path(&self, path: &DiagnosticPath) -> bool {
        self.selections
            .iter()
            .any(|selection| selection.path == *path)
    }
}

impl Drop for MetricSubscription {
    fn drop(&mut self) {
        self.ledger.release(self.id);
        *recording::lock(&self.status) = SubscriptionStatus::Ended;
    }
}

/// Validates all selections before acquiring any collection demand.
///
/// # Errors
/// Rejects a missing service, unknown paths, unsupported statistics and retention
/// above the runtime limits in [`crate::MetricsSettings`].
pub fn subscribe_metrics(
    world: &mut World,
    selections: &[MetricSelection],
) -> Result<MetricSubscription, MetricSubscriptionError> {
    let registry = world
        .get_resource::<MetricRegistry>()
        .ok_or(MetricSubscriptionError::ServiceNotInstalled)?;
    let settings = world.resource::<crate::MetricsSettings>();
    let mut capacities = HashMap::<DiagnosticPath, RetentionCapacity>::new();
    for selection in selections {
        let descriptor = if let Some(entry) = registry.entries.get(&selection.path) {
            entry.descriptor.clone()
        } else if world
            .resource::<DiagnosticsStore>()
            .get(&selection.path)
            .is_some()
        {
            definition::validate_native_path(selection.path.as_str())
                .map_err(|_| MetricSubscriptionError::UnknownPath)?;
            crate::MetricDescriptor::discovered(selection.path.clone())
        } else {
            return Err(MetricSubscriptionError::UnknownPath);
        };
        descriptor
            .result_unit(&selection.statistic)
            .map_err(MetricSubscriptionError::Capability)?;
        let capacity = retention_capacity(&selection.statistic, settings)?;
        capacities
            .entry(selection.path.clone())
            .and_modify(|current| *current = (*current).max(capacity))
            .or_insert(capacity);
    }
    let boundaries = Arc::new(Mutex::new(HashMap::new()));
    let ledger = &world.resource::<DemandLedger>().shared;
    let status = Arc::new(Mutex::new(SubscriptionStatus::PendingActivation));
    let subscription = ActiveSubscription {
        paths:      capacities.into_iter().collect(),
        status:     Arc::clone(&status),
        boundaries: Arc::clone(&boundaries),
    };
    let id = ledger.acquire(subscription);
    Ok(MetricSubscription {
        id,
        ledger: Arc::clone(ledger),
        status,
        boundaries,
        selections: selections.to_vec(),
    })
}

fn retention_capacity(
    statistic: &MetricStatistic,
    settings: &crate::MetricsSettings,
) -> Result<RetentionCapacity, MetricSubscriptionError> {
    let MetricStatistic::Window { window, reduction } = statistic else {
        return Ok(RetentionCapacity::new(LATEST_ONLY_CAPACITY));
    };
    let capacity = match window {
        SampleWindow::LastSamples(samples) => samples.get(),
        SampleWindow::Trailing { duration, capacity } => {
            if duration.is_zero() || *duration > settings.max_trailing {
                return Err(MetricSubscriptionError::RetentionExceeded);
            }
            capacity.get()
        },
    };
    let baseline = usize::from(matches!(
        reduction,
        WindowReduction::Delta | WindowReduction::Rate
    ));
    let capacity = capacity.saturating_add(baseline);
    if capacity > settings.max_retained_samples.get() {
        return Err(MetricSubscriptionError::RetentionExceeded);
    }
    Ok(RetentionCapacity::new(capacity))
}
