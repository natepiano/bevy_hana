use hana_rigging::DeviceKind;

fn classify(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Display => "display",
        DeviceKind::Camera => "camera",
        DeviceKind::AudioInterface => "audio interface",
        DeviceKind::DmxUniverse => "DMX universe",
        DeviceKind::HidPanel => "HID panel",
    }
}

fn main() {
    let _ = classify(DeviceKind::Display);
}
