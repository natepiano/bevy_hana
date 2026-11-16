use super::BoundaryCoverage;
use super::ObservedBoundaries;
use super::RequestedWindow;
use super::SequenceRange;
/// Numeric evidence and provenance accompanying a reading.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowCoverage {
    /// Evidence boundary requested by the consumer.
    pub requested:    RequestedWindow,
    /// Measurement endpoints represented by retained samples.
    pub observed:     ObservedBoundaries,
    /// Inclusive accepted sequence identities in the snapshot.
    pub sequences:    SequenceRange,
    /// Number of observations included in the result.
    pub sample_count: usize,
    /// Whether the full requested boundary is retained.
    pub boundary:     BoundaryCoverage,
}
