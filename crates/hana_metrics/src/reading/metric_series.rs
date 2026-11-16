use std::time::Duration;

use super::MetricWindow;
use super::UnavailableReason;
/// Availability of a numeric series snapshot.
#[derive(Clone, Debug, PartialEq)]
pub enum MetricSeries {
    /// Complete evidence meeting the declared freshness policy.
    Available(MetricWindow),
    /// Complete evidence whose last sample is older than its live deadline.
    Stale {
        /// Last complete snapshot.
        last: MetricWindow,
        /// Elapsed run time since the last sample endpoint.
        age:  Duration,
    },
    /// A named reason for the absence of a usable value.
    Unavailable(UnavailableReason),
}
