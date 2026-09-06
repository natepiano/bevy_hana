//! A driver states what its transport carried. It cannot assert an arrival.
//!
//! Both errors below are the same defect the flow axis kept growing back: a device claiming a
//! datum it never delivered, and so reaching the state every presenting reading is derived from.

use std::time::Instant;

use hana_rigging::prelude::DeviceTransport;
use hana_rigging::prelude::SessionDatumArrivalEvidence;
use hana_rigging::prelude::SessionLease;
use hana_rigging::prelude::TransportObservation;

/// A sample from a display on which nothing moved: alive, and carrying no picture.
struct StillDisplaySample;

struct StillDisplayTransport;

impl DeviceTransport for StillDisplayTransport {
    type Observation = StillDisplaySample;

    fn classify(_still: &Self::Observation) -> TransportObservation {
        TransportObservation::ActivityWithoutDatum
    }
}

/// Rejected: the lease has no method that credits an arrival on a driver's say-so.
fn assert_an_arrival<Configuration>(lease: &mut SessionLease<Configuration>) {
    lease.record_datum_arrival();
}

/// Rejected: the evidence a lease carries cannot be spelled outside the kernel either.
fn spell_an_arrival(observed_at: Instant) -> SessionDatumArrivalEvidence {
    SessionDatumArrivalEvidence::ObservedAt(observed_at)
}

fn main() {}
