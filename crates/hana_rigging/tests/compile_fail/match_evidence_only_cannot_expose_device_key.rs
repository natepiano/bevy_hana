use hana_rigging::ReportedAs;

fn main() {
    let reported_as = ReportedAs::MatchEvidenceOnly;
    let ReportedAs::MatchEvidenceOnly(_) = reported_as;
}
