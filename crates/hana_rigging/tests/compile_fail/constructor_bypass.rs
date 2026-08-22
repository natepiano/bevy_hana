use hana_rigging::DeviceId;
use hana_rigging::Digest;
use hana_rigging::ReportedId;
use hana_rigging::SchemeName;

fn main() {
    let _ = DeviceId(1);
    let _ = SchemeName("edid-serial".to_owned());
    let _ = ReportedId("DELL-U2723QE-9J4K2H3".to_owned());
    let _ = Digest(14_695_981_039_346_656_037);
}
