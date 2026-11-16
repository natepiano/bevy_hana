//! Exact counters, preserving external source identity across observation gaps.

use super::MeasurementFailure;
use crate::CumulativeCounterSample;
use crate::ExternalCounterEpoch;
use crate::InvalidMetricSample;
use crate::RunOffset;
use crate::definition::CounterRecording;
use crate::definition::MetricRecording;

pub(crate) enum CounterState {
    NotCounter,
    Increments(u64),
    AwaitingTotal,
    External {
        epoch:    ExternalCounterEpoch,
        total:    u64,
        at:       RunOffset,
        sequence: u64,
    },
    Overflowed,
}

pub(crate) enum CounterEpochChange {
    Continued,
    Restarted,
}

impl CounterState {
    pub(super) const fn new(recording: MetricRecording) -> Self {
        match recording {
            MetricRecording::Counter(CounterRecording::Increment) => Self::Increments(0),
            MetricRecording::Counter(CounterRecording::Cumulative) => Self::AwaitingTotal,
            MetricRecording::Gauge | MetricRecording::TimingSpan => Self::NotCounter,
        }
    }

    pub(super) const fn restart_observation(&mut self) {
        if matches!(self, Self::Increments(_) | Self::Overflowed) {
            *self = Self::Increments(0);
        }
    }

    pub(super) const fn increment(&mut self, delta: u64) -> Result<u64, MeasurementFailure> {
        match *self {
            Self::Increments(total) => {
                if let Some(total) = total.checked_add(delta) {
                    *self = Self::Increments(total);
                    Ok(total)
                } else {
                    *self = Self::Overflowed;
                    Err(MeasurementFailure::CounterOverflow)
                }
            },
            Self::Overflowed => Err(MeasurementFailure::CounterOverflow),
            Self::NotCounter | Self::AwaitingTotal | Self::External { .. } => Err(
                MeasurementFailure::InvalidSample(InvalidMetricSample::WrongRecordingMethod),
            ),
        }
    }

    pub(super) fn record_total(
        &mut self,
        sample: CumulativeCounterSample,
        at: RunOffset,
        sequence: u64,
    ) -> Result<CounterEpochChange, MeasurementFailure> {
        let change = match *self {
            Self::AwaitingTotal => CounterEpochChange::Continued,
            Self::External { epoch, total, .. } => {
                if sample.epoch.generation() < epoch.generation() {
                    return Err(MeasurementFailure::ClockDomain);
                }
                if epoch == sample.epoch {
                    if sample.total < total {
                        return Err(MeasurementFailure::FallingTotalInEpoch);
                    }
                    CounterEpochChange::Continued
                } else {
                    CounterEpochChange::Restarted
                }
            },
            Self::NotCounter | Self::Increments(_) | Self::Overflowed => {
                return Err(MeasurementFailure::InvalidSample(
                    InvalidMetricSample::WrongRecordingMethod,
                ));
            },
        };
        *self = Self::External {
            epoch: sample.epoch,
            total: sample.total,
            at,
            sequence,
        };
        Ok(change)
    }
}
