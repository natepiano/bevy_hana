/// Accepted append-stream identities represented in a snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceRange {
    /// The snapshot contains no sample identities.
    Empty,
    /// Inclusive first and last accepted sequence identities.
    Inclusive {
        /// Earliest included sample identity.
        first: u64,
        /// Latest included sample identity.
        last:  u64,
    },
}
