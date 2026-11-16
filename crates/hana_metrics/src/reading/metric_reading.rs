use std::time::Duration;

use super::MetricEstimate;
use super::UnavailableReason;
/// Availability of a numeric scalar estimate.
#[derive(Clone, Debug, PartialEq)]
pub enum MetricReading {
    /// Complete evidence meeting the declared freshness policy.
    Available(MetricEstimate),
    /// Complete evidence whose last sample is older than its live deadline.
    Stale {
        /// Last complete estimate.
        last: MetricEstimate,
        /// Elapsed run time since the last sample endpoint.
        age:  Duration,
    },
    /// A named reason for the absence of a usable value.
    Unavailable(UnavailableReason),
}
