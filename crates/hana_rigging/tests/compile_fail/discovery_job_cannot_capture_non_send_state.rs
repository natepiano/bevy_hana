use std::rc::Rc;

use hana_rigging::DeviceScan;
use hana_rigging::DiscoveryJob;

fn main() {
    let non_send_state = Rc::new(());
    let _ = DiscoveryJob::new(move |_| {
        let _ = Rc::strong_count(&non_send_state);
        DeviceScan::Complete(Vec::new())
    });
}
