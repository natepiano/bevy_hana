use bevy::prelude::World;
use hana_rigging::DeviceScan;
use hana_rigging::DiscoveryJob;

fn main() {
    let _ = DiscoveryJob::new(|_: &mut World| DeviceScan::Complete(Vec::new()));
}
