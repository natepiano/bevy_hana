use hana_rigging::IdentityVerdict;

fn classify(identity_verdict: IdentityVerdict) -> &'static str {
    match identity_verdict {
        IdentityVerdict::Proven => "proven",
        IdentityVerdict::RestoreOnly => "restore only",
        IdentityVerdict::Authored => "authored",
        IdentityVerdict::Displaced { .. } => "displaced",
        IdentityVerdict::WrongUnit { .. } => "wrong unit",
        IdentityVerdict::Unverified(_) => "unverified",
    }
}

fn main() {
    let _ = classify(IdentityVerdict::Proven);
}
