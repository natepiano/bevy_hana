//! Dormant typed recorders and their shared append buffer.

use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::MeasurementFailure;
use super::ObservationClock;
use super::ObservationExtent;
use crate::Counter;
use crate::CumulativeCounterSample;
use crate::FrameContext;
use crate::Gauge;
use crate::IntervalGaugeSample;
use crate::InvalidMetricSample;
use crate::MetricKind;
use crate::ObservationContext;
use crate::ObservationEpoch;
use crate::ObservationInterval;
use crate::OperationContext;
use crate::TimingSpan;

/// Cloneable, thread-safe producer whose dormant path performs one gate load.
pub struct MetricRecorder<K: MetricKind> {
    pub(super) shared: Arc<RecorderState>,
    context:           ObservationContext,
    marker:            PhantomData<K>,
}

impl<K: MetricKind> Clone for MetricRecorder<K> {
    fn clone(&self) -> Self {
        Self {
            shared:  Arc::clone(&self.shared),
            context: self.context,
            marker:  PhantomData,
        }
    }
}

impl<K: MetricKind> MetricRecorder<K> {
    pub(super) const fn new(shared: Arc<RecorderState>) -> Self {
        Self {
            shared,
            context: ObservationContext {
                frame:     FrameContext::OutsideFrame,
                operation: OperationContext::NoOperation,
            },
            marker: PhantomData,
        }
    }

    /// Returns a recorder carrying the supplied frame and operation attribution.
    #[must_use]
    pub fn in_context(&self, context: ObservationContext) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            context,
            marker: PhantomData,
        }
    }

    fn push(&self, epoch: ObservationEpoch, extent: ObservationExtent, value: PendingValue) {
        let mut pending = lock(&self.shared.pending);
        pending.samples.push(PendingObservation {
            epoch,
            extent,
            value,
            context: self.context,
        });
    }

    fn epoch(&self) -> ObservationEpoch { lock(&self.shared.pending).epoch }

    fn invalid(&self, epoch: ObservationEpoch, failure: MeasurementFailure) {
        self.push(
            epoch,
            ObservationExtent::Instant(self.shared.clock.now()),
            PendingValue::Rejected(failure),
        );
    }
}

impl MetricRecorder<Gauge> {
    /// Records a finite instantaneous level only while collection is enabled.
    pub fn record_with(&self, value: impl FnOnce() -> f64) {
        if !self.shared.enabled.load(Ordering::Acquire) {
            return;
        }
        let epoch = self.epoch();
        let value = value();
        self.push(
            epoch,
            ObservationExtent::Instant(self.shared.clock.now()),
            PendingValue::Gauge(value),
        );
    }

    /// Records a summary with its actual source endpoints while enabled.
    pub fn record_interval_with(
        &self,
        value: impl FnOnce(&ObservationClock) -> Result<IntervalGaugeSample, InvalidMetricSample>,
    ) {
        if !self.shared.enabled.load(Ordering::Acquire) {
            return;
        }
        let epoch = self.epoch();
        match value(&self.shared.clock) {
            Ok(sample) => self.push(
                epoch,
                ObservationExtent::Interval(sample.interval()),
                PendingValue::Gauge(sample.value()),
            ),
            Err(error) => self.invalid(epoch, MeasurementFailure::InvalidSample(error)),
        }
    }
}

impl MetricRecorder<Counter> {
    /// Appends an exact increment while collection is enabled.
    pub fn increment_with(&self, delta: impl FnOnce() -> u64) {
        if !self.shared.enabled.load(Ordering::Acquire) {
            return;
        }
        let epoch = self.epoch();
        let delta = delta();
        self.push(
            epoch,
            ObservationExtent::Instant(self.shared.clock.now()),
            PendingValue::Increment(delta),
        );
    }

    /// Appends a source-owned cumulative total while collection is enabled.
    pub fn record_total_with(&self, total: impl FnOnce() -> CumulativeCounterSample) {
        if !self.shared.enabled.load(Ordering::Acquire) {
            return;
        }
        let epoch = self.epoch();
        let total = total();
        self.push(
            epoch,
            ObservationExtent::Instant(self.shared.clock.now()),
            PendingValue::Total(total),
        );
    }
}

impl MetricRecorder<TimingSpan> {
    /// Executes work once, measuring endpoints only while collection is enabled.
    pub fn measure<R>(&self, work: impl FnOnce() -> R) -> R {
        if !self.shared.enabled.load(Ordering::Acquire) {
            return work();
        }
        let epoch = self.epoch();
        let started = self.shared.clock.now();
        let mut guard = SpanCompletion {
            recorder: self,
            epoch,
            state: SpanCompletionState::Measuring,
        };
        let result = work();
        let ended = self.shared.clock.now();
        guard.state = SpanCompletionState::Completed;
        match ObservationInterval::new(started, ended) {
            Ok(interval) => self.push(
                epoch,
                ObservationExtent::Interval(interval),
                PendingValue::Duration(interval.duration()),
            ),
            Err(_) => self.invalid(epoch, MeasurementFailure::ClockDomain),
        }
        result
    }

    /// Records an elapsed duration with completion-only temporal attribution.
    pub fn record_duration_with(&self, elapsed: impl FnOnce() -> Duration) {
        if !self.shared.enabled.load(Ordering::Acquire) {
            return;
        }
        let epoch = self.epoch();
        let elapsed = elapsed();
        self.push(
            epoch,
            ObservationExtent::CompletionOnly(self.shared.clock.now()),
            PendingValue::Duration(elapsed),
        );
    }
}

struct SpanCompletion<'a> {
    recorder: &'a MetricRecorder<TimingSpan>,
    epoch:    ObservationEpoch,
    state:    SpanCompletionState,
}

enum SpanCompletionState {
    Measuring,
    Completed,
}

impl Drop for SpanCompletion<'_> {
    fn drop(&mut self) {
        if matches!(self.state, SpanCompletionState::Measuring) {
            self.recorder
                .invalid(self.epoch, MeasurementFailure::Aborted);
        }
    }
}

pub(crate) struct RecorderState {
    pub(super) enabled: AtomicBool,
    pub(super) pending: Mutex<PendingRecords>,
    pub(super) clock:   ObservationClock,
}

pub(crate) struct PendingRecords {
    pub(super) epoch:   ObservationEpoch,
    pub(super) samples: Vec<PendingObservation>,
}

#[derive(Clone, Copy)]
pub(crate) struct PendingObservation {
    pub(super) epoch:   ObservationEpoch,
    pub(super) extent:  ObservationExtent,
    pub(super) value:   PendingValue,
    pub(super) context: ObservationContext,
}

#[derive(Clone, Copy)]
pub(crate) enum PendingValue {
    Gauge(f64),
    Increment(u64),
    Total(CumulativeCounterSample),
    Duration(Duration),
    Rejected(MeasurementFailure),
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
