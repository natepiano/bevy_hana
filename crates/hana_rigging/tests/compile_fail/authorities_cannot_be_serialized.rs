use bevy::prelude::Component;
use bevy::prelude::Reflect;
use hana_rigging::AttemptCompletion;
use hana_rigging::SessionLease;

#[derive(Component, Reflect)]
struct Configuration;

fn serialize_authorities(
    completion: &AttemptCompletion<Configuration>,
    lease: &SessionLease<Configuration>,
) {
    let _ = serde_json::to_string(completion);
    let _ = serde_json::to_string(lease);
}

fn main() {}
