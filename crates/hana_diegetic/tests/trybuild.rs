//! Compile-time coverage for the public layout builder API: the typestate
//! helpers that must compile, and the misuses that must not.

#[test]
#[ignore = "slow compile-time API test; run when changing widget state or tooltip typestates"]
fn widget_state_and_tooltip_typestate_signatures_compile() {
    let test_cases = trybuild::TestCases::new();
    test_cases.pass("tests/trybuild/pass/typestate_helpers.rs");
    test_cases.pass("tests/trybuild/pass/tooltip_typestate.rs");
    test_cases.pass("tests/trybuild/pass/widget_state_appearance.rs");
    test_cases.compile_fail("tests/trybuild/fail/overlay_*.rs");
    test_cases.compile_fail("tests/trybuild/fail/editable_widget_*.rs");
    test_cases.compile_fail("tests/trybuild/fail/tooltip_*.rs");
    test_cases.compile_fail("tests/trybuild/fail/widget_state_appearance_*.rs");
    test_cases.compile_fail("tests/trybuild/fail/widget_state_verbs_*.rs");
}
