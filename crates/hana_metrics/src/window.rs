//! Validated selection windows and reduction requests.

use std::num::NonZeroUsize;
use std::time::Duration;

use hana_kana::ToF64;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::de::Error as _;

use crate::InvalidPercentile;
use crate::MeasurementFailure;
use crate::MetricNumber;

/// Finite percentile fraction in the inclusive range `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Percentile(f64);

impl Percentile {
    /// Validates a percentile fraction.
    ///
    /// # Errors
    /// Rejects non-finite fractions and fractions outside `[0, 1]`.
    pub fn try_new(fraction: f64) -> Result<Self, InvalidPercentile> {
        if !fraction.is_finite() {
            return Err(InvalidPercentile::NonFinite);
        }
        if !(0.0..=1.0).contains(&fraction) {
            return Err(InvalidPercentile::OutOfRange);
        }
        Ok(Self(fraction))
    }

    /// Returns the validated fraction.
    #[must_use]
    pub const fn fraction(self) -> f64 { self.0 }
}

impl<'de> Deserialize<'de> for Percentile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_new(f64::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// Exact retained-sample bounds requested for a reading.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SampleWindow {
    /// Most recent nonzero number of retained samples.
    LastSamples(NonZeroUsize),
    /// Samples within a trailing time range, with a bounded retained capacity.
    Trailing {
        /// Time range ending at the reading's endpoint.
        duration: Duration,
        /// Maximum number of samples retained for this selection.
        capacity: NonZeroUsize,
    },
}

/// Reduction requested over an exact retained-sample window.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum WindowReduction {
    /// Arithmetic mean of retained samples.
    Mean,
    /// Median of retained samples.
    Median,
    /// Largest retained sample.
    Maximum,
    /// Sum of additive observations.
    Sum,
    /// Counter change within one epoch and a measured baseline.
    Delta,
    /// Counter change divided by the measured baseline interval.
    Rate,
    /// Exact percentile of retained samples.
    Percentile(Percentile),
}

/// Reduces finite scalar samples using bounded arithmetic sample statistics.
pub(crate) fn reduce(
    samples: &[crate::RetainedObservation],
    reduction: WindowReduction,
) -> Result<crate::MetricNumber, crate::MeasurementFailure> {
    // Integer sample positions must remain exactly representable for rank comparisons.
    let count = u64::try_from(samples.len()).map_err(|_| MeasurementFailure::NumericRange)?;
    if count == 0 || count > (1_u64 << f64::MANTISSA_DIGITS) {
        return Err(MeasurementFailure::NumericRange);
    }
    let mut values: Vec<f64> = samples
        .iter()
        .map(|sample| match sample.number {
            MetricNumber::Scalar(value) => value,
            MetricNumber::Count(total) => total.to_f64(),
        })
        .collect();
    let value = match reduction {
        WindowReduction::Mean => {
            let divisor = values.len().to_f64();
            values.iter().map(|value| value / divisor).sum()
        },
        WindowReduction::Sum => values.iter().sum(),
        WindowReduction::Maximum => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        WindowReduction::Median | WindowReduction::Percentile(_) => {
            values.sort_by(f64::total_cmp);
            if let WindowReduction::Percentile(percentile) = reduction {
                // The checked sample count keeps integer positions exact. Compare
                // positions to the rounded rank without a narrowing numeric cast.
                let rank = (percentile.fraction() * values.len().to_f64())
                    .ceil()
                    .max(1.0);
                values
                    .iter()
                    .enumerate()
                    .find(|(index, _)| (index + 1).to_f64() >= rank)
                    .map_or(f64::NAN, |(_, value)| *value)
            } else {
                let middle = values.len() / 2;
                if values.len().is_multiple_of(2) {
                    values[middle].mul_add(0.5, values[middle - 1] * 0.5)
                } else {
                    values[middle]
                }
            }
        },
        WindowReduction::Delta | WindowReduction::Rate => {
            return Err(MeasurementFailure::NumericRange);
        },
    };
    if value.is_finite() {
        Ok(MetricNumber::Scalar(value))
    } else {
        Err(MeasurementFailure::NumericRange)
    }
}

pub(crate) fn contains(window: SampleWindow, at: crate::RunOffset, now: crate::RunOffset) -> bool {
    match window {
        SampleWindow::LastSamples(_) => at <= now,
        SampleWindow::Trailing { duration, .. } => {
            at.duration() > now.duration().saturating_sub(duration) && at <= now
        },
    }
}
