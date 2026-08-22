//! Compile-time coverage for Fairy Dust's first-step asset-root typestate.

#[test]
#[ignore = "slow compile-time API test; run when changing Fairy Dust builder typestates"]
fn asset_root_is_only_available_before_baseline_installation() {
    let test_cases = trybuild::TestCases::new();
    test_cases.pass("tests/trybuild/pass/asset_root_first.rs");
    test_cases.compile_fail("tests/trybuild/fail/asset_root_after_capability.rs");
    test_cases.compile_fail("tests/trybuild/fail/app_mut_while_pending.rs");
}
