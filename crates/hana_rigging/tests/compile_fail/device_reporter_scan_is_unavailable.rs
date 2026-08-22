use bevy::prelude::World;
use hana_rigging::DeviceReporter;
use hana_rigging::DeviceScan;

struct Reporter;

impl DeviceReporter for Reporter {
    fn scan(&mut self, _: &mut World) -> DeviceScan { DeviceScan::Complete(Vec::new()) }
}

fn main() {}
