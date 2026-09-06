use bevy::prelude::Component;
use bevy::prelude::Reflect;
use hana_rigging::AttemptCompletion;
use hana_rigging::SessionLease;

#[derive(Component, Reflect)]
struct Configuration;

fn main() {
    let _ = AttemptCompletion::<Configuration> {};
    let _ = SessionLease::<Configuration> {};
}
