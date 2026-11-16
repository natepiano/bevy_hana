use super::WindowCoverage;
use crate::MetricNumber;
use crate::MetricUnit;
use crate::RunOffset;
/// One typed scalar result with its original measurement endpoint and coverage.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricEstimate {
    /// Scalar result or exact integer counter value.
    pub value:       MetricNumber,
    /// Unit determined by the descriptor and requested statistic.
    pub unit:        MetricUnit,
    /// Original endpoint of the most recent included measurement.
    pub measured_at: RunOffset,
    /// Evidence used to compute this estimate.
    pub coverage:    WindowCoverage,
}
