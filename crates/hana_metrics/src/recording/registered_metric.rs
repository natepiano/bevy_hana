//! A registered descriptor and its owned or metadata-only source state.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use bevy_diagnostic::DiagnosticsStore;

use super::CounterEpochChange;
use super::CounterState;
use super::FilterState;
use super::MeasurementFailure;
use super::MetricQuality;
use super::ObservationClock;
use super::ObservationExtent;
use super::PendingObservation;
use super::PendingRecords;
use super::PendingValue;
use super::RecorderState;
use super::RetainedObservation;
use super::drain_metric;
use super::lock;
use crate::GaugeMeaning;
use crate::GaugeSemantics;
use crate::InvalidMetricSample;
use crate::MetricDescriptor;
use crate::MetricNumber;
use crate::ObservationEpoch;
use crate::RunOffset;
use crate::definition::MetricRecording;
use crate::demand::RetentionCapacity;
use crate::demand::SubscriptionId;

pub(crate) struct RegisteredMetric {
    pub(crate) descriptor:   MetricDescriptor,
    pub(crate) source:       MetricSource,
    pub(crate) epoch:        ObservationEpoch,
    pub(crate) effective_at: RunOffset,
    pub(crate) activation:   Arc<Mutex<EffectiveEpoch>>,
    pub(crate) demands:      HashMap<SubscriptionId, RetentionCapacity>,
    pub(crate) activity:     SourceActivity,
}

pub(crate) enum MetricSource {
    Owned(OwnedMetric),
    AdoptedNative,
    DiscoveredNative,
}

pub(crate) struct EffectiveEpoch {
    pub(crate) epoch:        ObservationEpoch,
    pub(crate) effective_at: RunOffset,
}

/// Whether a source is dormant, collecting, or permanently ended.
pub(crate) enum SourceActivity {
    /// No consumer currently requests collection.
    Dormant,
    /// An acknowledged consumer keeps this source collecting.
    Active,
    /// Test fixture for permanent source ending, implemented by the next phase.
    #[cfg(test)]
    Ended,
}

impl SourceActivity {
    pub(crate) const fn is_ended(&self) -> bool {
        match self {
            #[cfg(test)]
            Self::Ended => true,
            Self::Dormant | Self::Active => false,
        }
    }
}

pub(crate) struct OwnedMetric {
    pub(super) recorder:           Arc<RecorderState>,
    pub(crate) retained:           VecDeque<RetainedObservation>,
    pub(crate) quality:            MetricQuality,
    pub(crate) counter:            CounterState,
    pub(crate) smoothing:          FilterState,
    pub(crate) evicted:            EvictionHistory,
    pub(crate) sequence:           u64,
    pub(super) retention_capacity: RetentionCapacity,
    last_endpoint:                 EndpointHistory,
    pub(super) spare:              Vec<PendingObservation>,
}

enum EndpointHistory {
    Empty,
    Last(RunOffset),
}

/// Whether retained evidence has been discarded in the current epoch.
#[derive(Clone, Copy, Debug)]
pub(crate) enum EvictionHistory {
    None,
    Through { at: RunOffset, sequence: u64 },
}

impl RegisteredMetric {
    pub(super) fn owned(
        descriptor: MetricDescriptor,
        clock: ObservationClock,
        recording: MetricRecording,
    ) -> (Self, Arc<RecorderState>) {
        let activation = Arc::new(Mutex::new(EffectiveEpoch {
            epoch:        ObservationEpoch::initial(),
            effective_at: RunOffset::default(),
        }));
        let recorder = Arc::new(RecorderState {
            enabled: AtomicBool::new(false),
            clock,
            pending: Mutex::new(PendingRecords {
                epoch:   ObservationEpoch::initial(),
                samples: Vec::new(),
            }),
        });
        let source = MetricSource::Owned(OwnedMetric {
            recorder:           Arc::clone(&recorder),
            retained:           VecDeque::new(),
            quality:            MetricQuality::Complete,
            counter:            CounterState::new(recording),
            smoothing:          FilterState::Unseeded,
            evicted:            EvictionHistory::None,
            sequence:           0,
            retention_capacity: RetentionCapacity::latest(),
            last_endpoint:      EndpointHistory::Empty,
            spare:              Vec::new(),
        });
        (
            Self {
                descriptor,
                source,
                epoch: ObservationEpoch::initial(),
                effective_at: RunOffset::default(),
                activation,
                demands: HashMap::new(),
                activity: SourceActivity::Dormant,
            },
            recorder,
        )
    }

    pub(super) fn adopted(descriptor: MetricDescriptor) -> Self {
        Self {
            descriptor,
            source: MetricSource::AdoptedNative,
            epoch: ObservationEpoch::initial(),
            effective_at: RunOffset::default(),
            activation: Arc::new(Mutex::new(EffectiveEpoch {
                epoch:        ObservationEpoch::initial(),
                effective_at: RunOffset::default(),
            })),
            demands: HashMap::new(),
            activity: SourceActivity::Dormant,
        }
    }

    pub(crate) fn discovered(descriptor: MetricDescriptor) -> Self {
        let mut entry = Self::adopted(descriptor);
        entry.source = MetricSource::DiscoveredNative;
        entry
    }

    pub(crate) const fn is_active(&self) -> bool { matches!(self.activity, SourceActivity::Active) }

    fn start_epoch(&mut self, at: RunOffset) {
        self.epoch = self.epoch.next();
        self.effective_at = at;
        *lock(&self.activation) = EffectiveEpoch {
            epoch:        self.epoch,
            effective_at: at,
        };
        if let MetricSource::Owned(owned) = &mut self.source {
            owned.retained.clear();
            owned.evicted = EvictionHistory::None;
            owned.quality = MetricQuality::Complete;
            owned.smoothing = FilterState::Unseeded;
            owned.last_endpoint = EndpointHistory::Empty;
        }
    }

    pub(crate) fn activate(&mut self, at: RunOffset, capacity: RetentionCapacity) {
        self.start_epoch(at);
        self.activity = SourceActivity::Active;
        if let MetricSource::Owned(owned) = &mut self.source {
            owned.counter.restart_observation();
            owned.retention_capacity = capacity;
            owned.retained = VecDeque::with_capacity(capacity.get());
            let mut pending = lock(&owned.recorder.pending);
            pending.samples.clear();
            pending.epoch = self.epoch;
            drop(pending);
            owned.recorder.enabled.store(true, Ordering::Release);
        }
    }

    pub(crate) fn release(&mut self, store: &mut DiagnosticsStore, clock: &ObservationClock) {
        if let MetricSource::Owned(owned) = &self.source {
            owned.recorder.enabled.store(false, Ordering::Release);
        }
        drain_metric(self, store, clock);
        if let MetricSource::Owned(owned) = &mut self.source {
            owned.retained = VecDeque::new();
        }
        self.activity = SourceActivity::Dormant;
    }

    pub(crate) fn resize_retention(&mut self, capacity: RetentionCapacity) {
        if let MetricSource::Owned(owned) = &mut self.source {
            owned.retention_capacity = capacity;
            while owned.retained.len() > capacity.get() {
                if let Some(sample) = owned.retained.pop_front() {
                    owned.evicted = EvictionHistory::Through {
                        at:       sample.extent.end(),
                        sequence: sample.sequence,
                    };
                }
            }
            if owned.retained.capacity() < capacity.get() {
                owned
                    .retained
                    .reserve(capacity.get() - owned.retained.len());
            }
        }
    }

    pub(super) fn accept(
        &mut self,
        sample: PendingObservation,
        now: RunOffset,
        recording_epoch: ObservationEpoch,
    ) -> Result<RetainedObservation, MeasurementFailure> {
        let wrong_method =
            MeasurementFailure::InvalidSample(InvalidMetricSample::WrongRecordingMethod);
        let MetricSource::Owned(owned) = &mut self.source else {
            return Err(wrong_method);
        };
        owned.sequence = owned.sequence.saturating_add(1);
        if sample.epoch != recording_epoch {
            return Err(MeasurementFailure::ClockDomain);
        }
        let at = sample.extent.end();
        if at > now
            || matches!(owned.last_endpoint, EndpointHistory::Last(previous) if at < previous)
        {
            return Err(MeasurementFailure::ClockDomain);
        }
        let sequence = owned.sequence;
        let number = self.sample_number(sample.value, sample.extent)?;
        if let MetricSource::Owned(owned) = &mut self.source {
            owned.last_endpoint = EndpointHistory::Last(at);
        }
        Ok(RetainedObservation {
            sequence,
            extent: sample.extent,
            number,
            epoch: self.epoch,
            context: sample.context,
        })
    }

    fn sample_number(
        &mut self,
        value: PendingValue,
        extent: ObservationExtent,
    ) -> Result<MetricNumber, MeasurementFailure> {
        let wrong_method =
            MeasurementFailure::InvalidSample(InvalidMetricSample::WrongRecordingMethod);
        let MetricSource::Owned(owned) = &mut self.source else {
            return Err(wrong_method);
        };
        match value {
            PendingValue::Rejected(failure) => Err(failure),
            PendingValue::Gauge(value) => {
                if !value.is_finite() {
                    return Err(MeasurementFailure::NonFiniteValue);
                }
                match (self.descriptor.gauge_semantics(), extent) {
                    (
                        GaugeMeaning::Gauge(GaugeSemantics::IntervalSummary(_)),
                        ObservationExtent::Interval(_),
                    )
                    | (
                        GaugeMeaning::Gauge(GaugeSemantics::Level | GaugeSemantics::AdditiveSample),
                        ObservationExtent::Instant(_),
                    ) => Ok(MetricNumber::Scalar(value)),
                    (GaugeMeaning::Gauge(GaugeSemantics::IntervalSummary(_)), _) => Err(
                        MeasurementFailure::InvalidSample(InvalidMetricSample::MissingInterval),
                    ),
                    _ => Err(wrong_method),
                }
            },
            PendingValue::Duration(elapsed) => Ok(MetricNumber::Scalar(elapsed.as_secs_f64())),
            PendingValue::Increment(delta) => {
                owned.counter.increment(delta).map(MetricNumber::Count)
            },
            PendingValue::Total(sample) => {
                if matches!(
                    owned
                        .counter
                        .record_total(sample, extent.end(), owned.sequence)?,
                    CounterEpochChange::Restarted
                ) {
                    self.start_epoch(extent.end());
                }
                Ok(MetricNumber::Count(sample.total))
            },
        }
    }
}
