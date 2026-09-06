use hana_rigging::prelude::DeviceKind;

fn classify(device_kind: DeviceKind) -> &'static str {
    match device_kind {
        DeviceKind::Display => "display",
        DeviceKind::Camera => "camera",
        DeviceKind::AudioInterface => "audio interface",
        DeviceKind::DmxUniverse => "DMX universe",
    }
}

fn main() {
    let _ = classify(DeviceKind::Display);
}
