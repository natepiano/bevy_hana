use hana_rigging::ApplyPermit;

fn cannot_match(permit: ApplyPermit) {
    let ApplyPermit(_) = permit;
}

fn main() {}
