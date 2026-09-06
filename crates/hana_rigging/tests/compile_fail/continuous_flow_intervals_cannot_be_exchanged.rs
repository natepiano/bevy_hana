use std::time::Duration;

use hana_rigging::ContinuousFlowExpectation;
use hana_rigging::FirstDatumTimeout;
use hana_rigging::MaximumDatumGap;

fn continuous_flow_expectation(
    first_datum_timeout: FirstDatumTimeout,
    maximum_datum_gap: MaximumDatumGap,
) -> ContinuousFlowExpectation {
    ContinuousFlowExpectation::new(first_datum_timeout, maximum_datum_gap)
}

fn main() {
    let Ok(first_datum_timeout) = FirstDatumTimeout::new(Duration::from_secs(1)) else {
        return;
    };
    let Ok(maximum_datum_gap) = MaximumDatumGap::new(Duration::from_secs(1)) else {
        return;
    };

    let _ = continuous_flow_expectation(maximum_datum_gap, first_datum_timeout);
}
