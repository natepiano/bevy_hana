/// Requested evidence bounds, independent of how many samples arrived.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestedWindow {
    /// Number of most recent observations requested.
    Samples(usize),
    /// Time boundary evaluated at the consumer's read time.
    Trailing {
        /// Excluded opening boundary.
        start: crate::RunOffset,
        /// Included closing boundary.
        end:   crate::RunOffset,
    },
}
