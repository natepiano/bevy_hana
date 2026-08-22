use hana_rigging::DiscoveryLimits;

fn main() {
    let mut discovery_limits = DiscoveryLimits::default();
    discovery_limits.set_max_concurrent_jobs(0);
    discovery_limits.set_max_completions_per_frame(0);
}
