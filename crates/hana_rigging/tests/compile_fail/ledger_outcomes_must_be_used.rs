#![deny(unused_must_use)]

use bevy::prelude::Component;
use bevy::prelude::Reflect;
use hana_rigging::AttemptRef;
use hana_rigging::DriverAbortReason;
use hana_rigging::DriverLedger;
use hana_rigging::EstablishedContext;
use hana_rigging::Establishing;
use hana_rigging::RoleKey;

#[derive(Component, Reflect)]
struct Configuration;

fn establish_and_forget(
    ledger: &mut DriverLedger<Configuration>,
    context: EstablishedContext<'_, Configuration>,
) {
    ledger.establish_lease(context, Establishing::Live, |()| ());
}

fn abort_and_forget(ledger: &mut DriverLedger<Configuration>, attempt: AttemptRef) {
    ledger.abort_attempt(attempt, DriverAbortReason::OperationEnded);
}

fn discard_and_forget(ledger: &mut DriverLedger<Configuration>, role: &RoleKey) {
    ledger.discard_retained(role);
}

fn main() {}
