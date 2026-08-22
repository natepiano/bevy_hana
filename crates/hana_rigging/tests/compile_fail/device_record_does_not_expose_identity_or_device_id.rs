use hana_rigging::DeviceRecord;

fn main() {
    let device_record = device_record();
    let _ = device_record.identity;
    let _ = device_record.device_id;
    let _ = device_record.key;
}

fn device_record() -> DeviceRecord { loop {} }
