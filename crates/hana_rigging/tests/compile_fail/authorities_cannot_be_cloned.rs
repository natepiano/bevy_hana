use bevy::prelude::Component;
use bevy::prelude::Reflect;
use hana_rigging::AttemptCompletion;
use hana_rigging::SessionLease;

#[derive(Component, Reflect)]
struct Configuration;

fn clone_authorities(
    completion: &AttemptCompletion<Configuration>,
    lease: &SessionLease<Configuration>,
) {
    let _: AttemptCompletion<Configuration> = completion.clone();
    let _: SessionLease<Configuration> = lease.clone();
}

fn main() {}
