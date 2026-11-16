use bevy_diagnostic::DiagnosticPath;
use bevy_diagnostic::DiagnosticsStore;
use bevy_ecs::prelude::Res;
use bevy_ecs::system::SystemParam;
use hana_kana::ToF64;

use super::BoundaryCoverage;
use super::FreshnessState;
use super::MetricEstimate;
use super::MetricReading;
use super::MetricSeries;
use super::MetricWindow;
use super::ObservedBoundaries;
use super::RequestedWindow;
use super::SequenceRange;
use super::UnavailableReason;
use super::WindowCoverage;
use crate::FrameContext;
use crate::MeasurementFailure;
use crate::MetricCapability;
use crate::MetricCapabilityFailure;
use crate::MetricNumber;
use crate::MetricQuality;
use crate::MetricSelection;
use crate::MetricStatistic;
use crate::MetricSubscription;
use crate::ObservationContext;
use crate::ObservationExtent;
use crate::OperationContext;
use crate::RetainedObservation;
use crate::RunOffset;
use crate::SampleWindow;
use crate::WindowReduction;
use crate::demand::CounterBaseline;
use crate::demand::SubscriptionBoundary;
use crate::recording;
use crate::recording::EvictionHistory;
use crate::recording::FilterState;
use crate::recording::MetricRegistry;
use crate::recording::MetricSource;
use crate::recording::ObservationClock;
use crate::recording::RegisteredMetric;
use crate::window;

/// Reads numeric snapshots without changing subscription demand.
#[derive(SystemParam)]
pub struct MetricReader<'w> {
    registry: Res<'w, MetricRegistry>,
    store:    Res<'w, DiagnosticsStore>,
    clock:    Res<'w, ObservationClock>,
}

impl MetricReader<'_> {
    /// Reads a selection held by this subscription without acquiring demand.
    pub fn read(
        &self,
        subscription: &MetricSubscription,
        selection: &MetricSelection,
    ) -> MetricReading {
        let Some(entry) = self.registry.entries.get(&selection.path) else {
            return MetricReading::Unavailable(UnavailableReason::UnknownPath);
        };
        let unit = match entry.descriptor.result_unit(&selection.statistic) {
            Ok(unit) => unit,
            Err(failure) => {
                return MetricReading::Unavailable(UnavailableReason::Unsupported(failure));
            },
        };
        if !subscription.holds(selection) {
            return MetricReading::Unavailable(UnavailableReason::Dormant);
        }
        if let Err(reason) = ready(entry, subscription) {
            return MetricReading::Unavailable(reason);
        }
        let now = self.clock.now();
        let window = match selection.statistic {
            MetricStatistic::Window { window, .. } => window,
            _ => SampleWindow::LastSamples(std::num::NonZeroUsize::MIN),
        };
        let snapshot = match self.snapshot(entry, window, now) {
            Ok(snapshot) => snapshot,
            Err(reason) => return MetricReading::Unavailable(reason),
        };
        let value = match selection.statistic {
            MetricStatistic::Latest => snapshot
                .samples
                .last()
                .map(|sample| sample.number)
                .ok_or(UnavailableReason::NeverObserved),
            MetricStatistic::Smoothed => match &entry.source {
                MetricSource::Owned(owned) => match owned.smoothing {
                    FilterState::Seeded { value, .. } => Ok(MetricNumber::Scalar(value)),
                    FilterState::Unseeded => Err(UnavailableReason::NeverObserved),
                },
                _ => Err(UnavailableReason::Unsupported(
                    MetricCapabilityFailure::SmoothingDisabled,
                )),
            },
            MetricStatistic::Window {
                reduction: WindowReduction::Delta | WindowReduction::Rate,
                ..
            } => {
                let boundaries = recording::lock(&subscription.boundaries);
                if let Some(boundary) = boundaries.get(&selection.path) {
                    counter_reduction(entry, &snapshot, boundary, selection.statistic)
                } else {
                    Err(UnavailableReason::Dormant)
                }
            },
            MetricStatistic::Window { reduction, .. } => {
                window::reduce(&snapshot.samples, reduction).map_err(UnavailableReason::Incomplete)
            },
        };
        let value = match value {
            Ok(value) => value,
            Err(reason) => return MetricReading::Unavailable(reason),
        };
        let Some(last) = snapshot.samples.last() else {
            return MetricReading::Unavailable(UnavailableReason::NeverObserved);
        };
        let measured_at = last.extent.end();
        let estimate = MetricEstimate {
            value,
            unit,
            measured_at,
            coverage: snapshot.coverage,
        };
        match FreshnessState::at(entry.descriptor.freshness(), measured_at, now) {
            FreshnessState::Stale(age) => MetricReading::Stale {
                last: estimate,
                age,
            },
            FreshnessState::Current => MetricReading::Available(estimate),
        }
    }

    /// Copies a bounded numeric window for a path held by this subscription.
    #[must_use]
    pub fn series(
        &self,
        subscription: &MetricSubscription,
        path: &DiagnosticPath,
        window: &SampleWindow,
    ) -> MetricSeries {
        let Some(entry) = self.registry.entries.get(path) else {
            return MetricSeries::Unavailable(UnavailableReason::UnknownPath);
        };
        if entry.descriptor.capability() == MetricCapability::LatestOnly {
            return MetricSeries::Unavailable(UnavailableReason::Unsupported(
                MetricCapabilityFailure::LatestOnlySource,
            ));
        }
        if !subscription.holds_path(path) {
            return MetricSeries::Unavailable(UnavailableReason::Dormant);
        }
        if let Err(reason) = ready(entry, subscription) {
            return MetricSeries::Unavailable(reason);
        }
        let now = self.clock.now();
        match self.snapshot(entry, *window, now) {
            Err(reason) => MetricSeries::Unavailable(reason),
            Ok(snapshot) => {
                let Some(last) = snapshot.samples.last() else {
                    return MetricSeries::Unavailable(UnavailableReason::NeverObserved);
                };
                match FreshnessState::at(entry.descriptor.freshness(), last.extent.end(), now) {
                    FreshnessState::Stale(age) => MetricSeries::Stale {
                        last: snapshot,
                        age,
                    },
                    FreshnessState::Current => MetricSeries::Available(snapshot),
                }
            },
        }
    }

    fn snapshot(
        &self,
        entry: &RegisteredMetric,
        window: SampleWindow,
        now: RunOffset,
    ) -> Result<MetricWindow, UnavailableReason> {
        let samples = match &entry.source {
            MetricSource::Owned(owned) => {
                if owned.retained.is_empty() {
                    return Err(UnavailableReason::NeverObserved);
                }
                match window {
                    SampleWindow::LastSamples(count) => {
                        let mut selected: Vec<_> = owned
                            .retained
                            .iter()
                            .rev()
                            .filter(|sample| sample.extent.end() <= now)
                            .take(count.get())
                            .copied()
                            .collect();
                        selected.reverse();
                        selected
                    },
                    SampleWindow::Trailing { .. } => owned
                        .retained
                        .iter()
                        .filter(|sample| window::contains(window, sample.extent.end(), now))
                        .copied()
                        .collect(),
                }
            },
            MetricSource::AdoptedNative | MetricSource::DiscoveredNative => {
                let measurement = self
                    .store
                    .get(entry.descriptor.path())
                    .and_then(bevy_diagnostic::Diagnostic::measurement)
                    .ok_or(UnavailableReason::NeverObserved)?;
                if !measurement.value.is_finite() {
                    return Err(UnavailableReason::Incomplete(
                        MeasurementFailure::NonFiniteValue,
                    ));
                }
                let elapsed = measurement
                    .time
                    .checked_duration_since(self.clock.origin)
                    .ok_or(UnavailableReason::Incomplete(
                        MeasurementFailure::ClockDomain,
                    ))?;
                vec![RetainedObservation {
                    sequence: 0,
                    extent:   ObservationExtent::Instant(RunOffset::new(elapsed)),
                    number:   MetricNumber::Scalar(measurement.value),
                    epoch:    entry.epoch,
                    context:  ObservationContext {
                        frame:     FrameContext::Independent,
                        operation: OperationContext::NoOperation,
                    },
                }]
            },
        };
        let snapshot = build_window(entry, samples, window, now);
        match snapshot.coverage.boundary {
            BoundaryCoverage::WarmingUp => Err(UnavailableReason::WarmingUp(snapshot.coverage)),
            BoundaryCoverage::CapacityLoss => Err(UnavailableReason::Incomplete(
                MeasurementFailure::CapacityLoss,
            )),
            BoundaryCoverage::Complete => Ok(snapshot),
        }
    }
}

fn ready(
    entry: &RegisteredMetric,
    subscription: &MetricSubscription,
) -> Result<(), UnavailableReason> {
    if !subscription.is_active() || !entry.is_active() && !entry.activity.is_ended() {
        return Err(UnavailableReason::Dormant);
    }
    if let MetricSource::Owned(owned) = &entry.source
        && let MetricQuality::Incomplete { first_failure, .. } = owned.quality
    {
        return Err(UnavailableReason::Incomplete(first_failure));
    }
    if entry.activity.is_ended() {
        return Err(UnavailableReason::SourceEnded(entry.epoch));
    }
    Ok(())
}

fn build_window(
    entry: &RegisteredMetric,
    samples: Vec<RetainedObservation>,
    window: SampleWindow,
    now: RunOffset,
) -> MetricWindow {
    let coverage = coverage(entry, &samples, window, now);
    MetricWindow {
        samples,
        unit: entry.descriptor.unit(),
        epoch: entry.epoch,
        coverage,
    }
}

fn coverage(
    entry: &RegisteredMetric,
    samples: &[RetainedObservation],
    window: SampleWindow,
    now: RunOffset,
) -> WindowCoverage {
    let requested = match window {
        SampleWindow::LastSamples(count) => RequestedWindow::Samples(count.get()),
        SampleWindow::Trailing { duration, .. } => RequestedWindow::Trailing {
            start: RunOffset::new(now.duration().saturating_sub(duration)),
            end:   now,
        },
    };
    let mut boundary = match window {
        SampleWindow::LastSamples(count) if samples.len() < count.get() => {
            BoundaryCoverage::WarmingUp
        },
        SampleWindow::Trailing { duration, .. }
            if now.duration() < duration
                || entry.effective_at.duration() > now.duration().saturating_sub(duration) =>
        {
            BoundaryCoverage::WarmingUp
        },
        _ => BoundaryCoverage::Complete,
    };
    if let MetricSource::Owned(owned) = &entry.source
        && let EvictionHistory::Through { at, sequence } = owned.evicted
    {
        let lost = match window {
            SampleWindow::Trailing { duration, .. } => {
                at.duration() > now.duration().saturating_sub(duration)
            },
            SampleWindow::LastSamples(count) => samples.len() < count.get() && sequence > 0,
        };
        if lost {
            boundary = BoundaryCoverage::CapacityLoss;
        }
    }
    let (observed, sequences) = match (samples.first(), samples.last()) {
        (Some(first), Some(last)) => (
            ObservedBoundaries::Observed {
                start: first.extent.end(),
                end:   last.extent.end(),
            },
            SequenceRange::Inclusive {
                first: first.sequence,
                last:  last.sequence,
            },
        ),
        _ => (ObservedBoundaries::Empty, SequenceRange::Empty),
    };
    WindowCoverage {
        requested,
        observed,
        sequences,
        sample_count: samples.len(),
        boundary,
    }
}

fn counter_reduction(
    entry: &RegisteredMetric,
    snapshot: &MetricWindow,
    boundary: &SubscriptionBoundary,
    statistic: MetricStatistic,
) -> Result<MetricNumber, UnavailableReason> {
    let Some(last) = snapshot.samples.last() else {
        return Err(UnavailableReason::NeverObserved);
    };
    let MetricNumber::Count(total) = last.number else {
        return Err(UnavailableReason::Unsupported(
            MetricCapabilityFailure::StatisticNotSupported,
        ));
    };
    let MetricSource::Owned(owned) = &entry.source else {
        return Err(UnavailableReason::Unsupported(
            MetricCapabilityFailure::LatestOnlySource,
        ));
    };
    let effective = if boundary.epoch == entry.epoch {
        boundary.effective_at
    } else {
        entry.effective_at
    };
    let window_start = match snapshot.coverage.requested {
        RequestedWindow::Trailing { start, .. } => start.max(effective),
        RequestedWindow::Samples(_) => snapshot
            .samples
            .first()
            .map_or(effective, |sample| sample.extent.end())
            .max(effective),
    };
    let baseline = if matches!(owned.counter, crate::recording::CounterState::Increments(_)) {
        increment_baseline(entry, snapshot, boundary, window_start, effective)
    } else if boundary.epoch == entry.epoch && window_start == effective {
        match boundary.baseline {
            CounterBaseline::Measured { .. } => boundary.baseline,
            CounterBaseline::AwaitingSnapshot => first_counter(owned.retained.iter(), effective),
        }
    } else {
        first_counter(snapshot.samples.iter(), window_start)
    };
    let CounterBaseline::Measured {
        total: baseline,
        at,
        ..
    } = baseline
    else {
        return Err(UnavailableReason::WarmingUp(snapshot.coverage.clone()));
    };
    let delta = total
        .checked_sub(baseline)
        .ok_or(UnavailableReason::Incomplete(
            MeasurementFailure::FallingTotalInEpoch,
        ))?;
    if matches!(
        statistic,
        MetricStatistic::Window {
            reduction: WindowReduction::Delta,
            ..
        }
    ) {
        return Ok(MetricNumber::Count(delta));
    }
    let elapsed = last.extent.end().duration().saturating_sub(at.duration());
    if elapsed.is_zero() {
        return Err(UnavailableReason::WarmingUp(snapshot.coverage.clone()));
    }
    let rate = delta.to_f64() / elapsed.as_secs_f64();
    if rate.is_finite() {
        Ok(MetricNumber::Scalar(rate))
    } else {
        Err(UnavailableReason::Incomplete(
            MeasurementFailure::NumericRange,
        ))
    }
}

fn precedes_increment_window(
    sample: &RetainedObservation,
    snapshot: &MetricWindow,
    window_start: RunOffset,
) -> bool {
    match snapshot.coverage.requested {
        RequestedWindow::Samples(_) => match snapshot.coverage.sequences {
            SequenceRange::Inclusive { first, .. } => sample.sequence < first,
            SequenceRange::Empty => false,
        },
        RequestedWindow::Trailing { .. } => sample.extent.end() <= window_start,
    }
}

fn increment_baseline(
    entry: &RegisteredMetric,
    snapshot: &MetricWindow,
    boundary: &SubscriptionBoundary,
    window_start: RunOffset,
    effective: RunOffset,
) -> CounterBaseline {
    let initial = if boundary.epoch == entry.epoch {
        boundary.baseline
    } else {
        CounterBaseline::AwaitingSnapshot
    };
    let minimum_sequence = match initial {
        CounterBaseline::Measured { sequence, .. } => sequence,
        CounterBaseline::AwaitingSnapshot => 0,
    };
    if let MetricSource::Owned(owned) = &entry.source
        && let Some(sample) = owned.retained.iter().rev().find(|sample| {
            sample.epoch == entry.epoch
                && sample.sequence >= minimum_sequence
                && sample.extent.end() >= effective
                && precedes_increment_window(sample, snapshot, window_start)
        })
    {
        return CounterBaseline::from_sample(sample);
    }
    match initial {
        CounterBaseline::Measured { .. } => initial,
        CounterBaseline::AwaitingSnapshot => CounterBaseline::Measured {
            total:    0,
            at:       effective,
            sequence: minimum_sequence,
        },
    }
}

fn first_counter<'a>(
    mut samples: impl Iterator<Item = &'a RetainedObservation>,
    start: RunOffset,
) -> CounterBaseline {
    samples.find(|sample| sample.extent.end() >= start).map_or(
        CounterBaseline::AwaitingSnapshot,
        CounterBaseline::from_sample,
    )
}
