use bevy::prelude::World;
use hana_rigging::DeviceReporter;
use hana_rigging::DiscoveryWork;

struct Reporter;

impl DeviceReporter for Reporter {
    fn discover(&mut self, _: &mut World) -> DiscoveryWork { todo!() }
}

fn main() {}
