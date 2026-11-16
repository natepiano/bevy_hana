use super::WindowCoverage;
use crate::MeasurementFailure;
use crate::MetricCapabilityFailure;
use crate::ObservationEpoch;
/// Why a reading cannot supply a value; unavailable states never carry values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    /// No measurement has arrived in the current epoch.
    NeverObserved,
    /// This subscription has no effective demand for the requested reading.
    Dormant,
    /// More time or samples are needed to cover the requested boundary.
    WarmingUp(WindowCoverage),
    /// The source has permanently ended the named observation epoch.
    SourceEnded(ObservationEpoch),
    /// The path is absent from the catalogue.
    UnknownPath,
    /// The source declaration cannot provide this statistic or history.
    Unsupported(MetricCapabilityFailure),
    /// Invalid or lost evidence prevents a complete result.
    Incomplete(MeasurementFailure),
}
