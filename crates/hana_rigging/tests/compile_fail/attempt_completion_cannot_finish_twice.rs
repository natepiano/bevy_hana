use bevy::prelude::Component;
use bevy::prelude::Reflect;
use hana_rigging::Applied;
use hana_rigging::AttemptCompletion;
use hana_rigging::DriverCompletion;

#[derive(Component, Reflect)]
struct Configuration;

fn finish_twice(completion: AttemptCompletion<Configuration>) {
    completion.finish(DriverCompletion::Succeeded(Applied::AsDispatched));
    completion.finish(DriverCompletion::Succeeded(Applied::AsDispatched));
}

fn main() {}
