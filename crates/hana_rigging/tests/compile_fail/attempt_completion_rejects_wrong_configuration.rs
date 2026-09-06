use bevy::prelude::Component;
use bevy::prelude::Reflect;
use hana_rigging::Applied;
use hana_rigging::AttemptCompletion;
use hana_rigging::DriverCompletion;

#[derive(Component, Reflect)]
struct ExpectedConfiguration;

#[derive(Component, Reflect)]
struct WrongConfiguration;

fn finish_with_wrong_configuration(completion: AttemptCompletion<ExpectedConfiguration>) {
    completion.finish(DriverCompletion::Succeeded(Applied::DiffersFromDispatched(
        WrongConfiguration,
    )));
}

fn main() {}
