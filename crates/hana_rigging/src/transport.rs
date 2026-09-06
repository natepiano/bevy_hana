//! The one flow question a hardware driver answers for itself.
//!
//! Screens, cameras, and every other device that binds a role reach the flow axis through the
//! same [`SessionLease`](crate::SessionLease). The transitions between flow states belong to the
//! kernel, where they are written once; a driver supplies only the judgment the kernel cannot
//! make, which is what its own transport just carried.

use std::time::Instant;

use crate::SessionDatumArrivalEvidence;

/// What one observation of a device transport carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportObservation {
    /// A datum reached a consumer.
    ///
    /// This is the only classification that can start a session flowing, and therefore the only
    /// one any presenting reading is ever derived from. Report it where the datum lands in front
    /// of a consumer, never where it is polled off the transport: a datum discarded between the
    /// two was presented to nobody.
    DatumDelivered,
    /// The transport was alive and carried no datum.
    ///
    /// Activity holds an already-delivering session open, because nothing having changed since
    /// the last datum means that datum is still the current one. It can never start a session
    /// flowing — the state it would have to reach has no method that accepts it.
    ActivityWithoutDatum,
    /// Nothing was observed, and the session's bounds are unaffected.
    Quiet,
}

/// One hardware driver's classification of its own transport observations.
///
/// A driver implements [`Self::classify`] and nothing else. Everything that follows — which flow
/// transition the classification permits, whether a session may begin flowing, what a consumer is
/// then told it can present — belongs to the kernel, and is written once in the flow state types
/// behind [`ContinuousFlowView`](crate::ContinuousFlowView).
///
/// The point of routing every device through here is traceability. A device that reports flow it
/// has nothing to show for is a defect in exactly one `classify` implementation, at exactly one
/// call site, rather than in whichever of a dozen scattered credit calls last got its ordering
/// wrong.
///
/// A device with no data axis at all — a window binding, say, whose expectation is
/// [`FlowExpectation::NotMonitored`](crate::FlowExpectation::NotMonitored) — has no transport to
/// classify and implements nothing here.
pub trait DeviceTransport {
    /// What one observation of this driver's transport yields.
    type Observation;

    /// Classify one observation. This is the whole driver-side contract.
    fn classify(observation: &Self::Observation) -> TransportObservation;

    /// Evidence for an observation made before this session's lease was issued.
    ///
    /// Supplied by the kernel; a driver overrides it. A session opened on activity alone has
    /// delivered nothing, so it begins awaiting its first datum exactly as one that observed
    /// nothing at all.
    fn pre_establishment_evidence(
        observation: &Self::Observation,
        observed_at: Instant,
    ) -> SessionDatumArrivalEvidence {
        SessionDatumArrivalEvidence::from_observation(Self::classify(observation), observed_at)
    }
}
