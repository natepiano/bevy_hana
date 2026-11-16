//! Accepted observations and the completeness of their temporal evidence.

use crate::InvalidMetricSample;
use crate::MetricNumber;
use crate::ObservationContext;
use crate::ObservationEpoch;
use crate::ObservationInterval;
use crate::RunOffset;

/// Why observations cannot establish complete measurement evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeasurementFailure {
    /// Producer supplied a sample incompatible with its declaration.
    InvalidSample(InvalidMetricSample),
    /// A scalar was not finite.
    NonFiniteValue,
    /// Exact integer accumulation overflowed.
    CounterOverflow,
    /// A source total fell without changing its reset identity.
    FallingTotalInEpoch,
    /// An endpoint or sequence crossed a clock discontinuity.
    ClockDomain,
    /// Work unwound before completing its timing record.
    Aborted,
    /// Retention discarded evidence inside a requested window.
    CapacityLoss,
    /// A numeric reduction exceeded the representable finite range.
    NumericRange,
}

/// Completeness of accepted evidence within the current observation epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricQuality {
    /// No invalid records have been encountered in this epoch.
    Complete,
    /// Invalid records leave a counted gap and preserve its first cause.
    Incomplete {
        /// Number of records rejected in this epoch.
        invalid_samples: u64,
        /// First failure, retained even if subsequent records succeed.
        first_failure:   MeasurementFailure,
    },
}

impl MetricQuality {
    pub(super) const fn reject(&mut self, failure: MeasurementFailure) {
        *self = match *self {
            Self::Complete => Self::Incomplete {
                invalid_samples: 1,
                first_failure:   failure,
            },
            Self::Incomplete {
                invalid_samples,
                first_failure,
            } => Self::Incomplete {
                invalid_samples: invalid_samples.saturating_add(1),
                first_failure,
            },
        };
    }
}

/// Temporal evidence accompanying one retained observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationExtent {
    /// Instantaneous gauge or exact counter snapshot.
    Instant(RunOffset),
    /// Actual endpoints of an interval summary or measured span.
    Interval(ObservationInterval),
    /// Completed duration whose start endpoint was not observed.
    CompletionOnly(RunOffset),
}

impl ObservationExtent {
    /// Returns the original measurement or completion endpoint.
    #[must_use]
    pub const fn end(self) -> RunOffset {
        match self {
            Self::Instant(end) | Self::CompletionOnly(end) => end,
            Self::Interval(interval) => interval.end(),
        }
    }
}

/// One accepted sample retained for exact bounded reading windows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RetainedObservation {
    /// Append-stream identity; gaps expose rejected observations.
    pub sequence: u64,
    /// Actual temporal attribution.
    pub extent:   ObservationExtent,
    /// Scalar value or exact running counter total.
    pub number:   MetricNumber,
    /// Observation epoch containing this sample.
    pub epoch:    ObservationEpoch,
    /// Producer-supplied frame and causal attribution.
    pub context:  ObservationContext,
}
