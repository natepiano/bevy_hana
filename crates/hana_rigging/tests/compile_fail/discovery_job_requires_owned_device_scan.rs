use hana_rigging::DeviceScan;
use hana_rigging::DiscoveryJob;

fn main() {
    let device_scan = DeviceScan::Complete(Vec::new());
    let _ = DiscoveryJob::new(move |_| &device_scan);
}
