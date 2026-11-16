/// Endpoints represented by observations in a snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservedBoundaries {
    /// No sample endpoints are represented.
    Empty,
    /// Range of included measurement endpoints.
    Observed {
        /// First included endpoint.
        start: crate::RunOffset,
        /// Last included endpoint.
        end:   crate::RunOffset,
    },
}
