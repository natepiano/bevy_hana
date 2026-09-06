use bevy::prelude::Component;
use bevy::prelude::Reflect;
use hana_rigging::AttemptCompletion;
use hana_rigging::DriverLedger;
use hana_rigging::RoleKey;
use hana_rigging::SessionLease;

#[derive(Component, Reflect)]
struct Configuration;

fn take_owned_authorities(ledger: &DriverLedger<Configuration>, role: &RoleKey) {
    let _: SessionLease<Configuration> = ledger.session_of(role);
    let _: AttemptCompletion<Configuration> = ledger.attempt_of(role);
}

fn main() {}
