/// Whether the requested boundary has complete retained evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryCoverage {
    /// The whole requested boundary is covered without known retention loss.
    Complete,
    /// Observation began too recently or too few samples have arrived.
    WarmingUp,
    /// Retention discarded evidence inside the requested boundary.
    CapacityLoss,
}
