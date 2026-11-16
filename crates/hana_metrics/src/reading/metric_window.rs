use super::WindowCoverage;
use crate::MetricUnit;
use crate::ObservationEpoch;
use crate::RetainedObservation;
/// Consumer-owned snapshot of ordered numeric observations.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricWindow {
    /// Ordered observations copied from the retained stream.
    pub samples:  Vec<RetainedObservation>,
    /// Base unit of each observation.
    pub unit:     MetricUnit,
    /// Continuous observation epoch containing these samples.
    pub epoch:    ObservationEpoch,
    /// Requested and observed evidence boundaries.
    pub coverage: WindowCoverage,
}
