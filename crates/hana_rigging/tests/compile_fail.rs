//! Compile-time API boundaries that reporters and endpoint drivers must not bypass.

use bevy::app::App;
use bevy::tasks::IoTaskPool;
use hana_rigging::prelude::DeviceReporter;
use hana_rigging::prelude::DeviceScan;
use hana_rigging::prelude::DiscoveryCadence;
use hana_rigging::prelude::DiscoveryJob;
use hana_rigging::prelude::DiscoveryWork;
use hana_rigging::prelude::FirstCompleteSetStatus;
use hana_rigging::prelude::MainThreadDiscoveryJob;
use hana_rigging::prelude::ReporterCoverage;
use hana_rigging::prelude::ReporterHealth;
use hana_rigging::prelude::ReporterId;
use hana_rigging::prelude::ReporterOutcomeHealth;
use hana_rigging::prelude::ReporterRegistration;
use hana_rigging::prelude::RiggingAppExt;
use hana_rigging::prelude::RiggingPlugin;

struct BackgroundReporter;

impl DeviceReporter for BackgroundReporter {
    fn discover(&mut self) -> DiscoveryWork {
        DiscoveryWork::Background(DiscoveryJob::new(|_| DeviceScan::Complete(Vec::new())))
    }
}

struct ImmediateReporter;

impl DeviceReporter for ImmediateReporter {
    fn discover(&mut self) -> DiscoveryWork {
        DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(|_| {
            DeviceScan::Complete(Vec::new())
        }))
    }
}

#[test]
fn required_background_reporter_stays_blocked_before_io_pool_initialization()
-> Result<(), &'static str> {
    assert!(IoTaskPool::try_get().is_none());
    let mut app = App::new();
    app.add_plugins(RiggingPlugin);
    let reporter = app.add_device_reporter(
        BackgroundReporter,
        ReporterRegistration::required(
            DiscoveryCadence::OnDemand,
            ReporterCoverage::MatchingEvidenceOnly,
            std::time::Duration::from_secs(10),
        ),
    );

    app.update();

    let health = reporter_health(&app, reporter).ok_or("reporter health was not projected")?;
    assert!(matches!(
        health.first_complete_set(),
        FirstCompleteSetStatus::Waiting(_)
    ));
    assert!(matches!(
        health.outcome(),
        ReporterOutcomeHealth::NotCompleted
    ));
    assert_eq!(health.completed_runs(), 0);
    Ok(())
}

#[test]
fn immediate_reporter_completes_without_io_pool_initialization() -> Result<(), &'static str> {
    assert!(IoTaskPool::try_get().is_none());
    let mut app = App::new();
    app.add_plugins(RiggingPlugin);
    let reporter = app.add_device_reporter(
        ImmediateReporter,
        ReporterRegistration::required(
            DiscoveryCadence::OnDemand,
            ReporterCoverage::MatchingEvidenceOnly,
            std::time::Duration::from_secs(10),
        ),
    );

    app.update();
    app.update();

    assert!(IoTaskPool::try_get().is_none());
    let health = reporter_health(&app, reporter).ok_or("reporter health was not projected")?;
    assert_eq!(health.completed_runs(), 1);
    assert!(matches!(
        health.outcome(),
        ReporterOutcomeHealth::Succeeded { .. }
    ));
    Ok(())
}

fn reporter_health(app: &App, reporter: ReporterId) -> Option<&ReporterHealth> {
    app.world()
        .iter_entities()
        .filter_map(|entity| entity.get::<ReporterHealth>())
        .find(|health| health.belongs_to(reporter))
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn constructor_cannot_bypass_validated_constructors() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/constructor_bypass.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn device_kind_match_must_cover_every_variant() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/device_kind_match_must_cover_every_variant.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn identity_verdict_requires_a_wildcard_arm() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/identity_verdict_requires_a_wildcard_arm.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn device_record_cannot_expose_reconciliation_results_or_a_reported_key() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/device_record_does_not_expose_identity_or_device_id.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn unreachable_presence_requires_since() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/presence_unreachable_requires_since.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn match_evidence_only_cannot_expose_a_device_key() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/match_evidence_only_cannot_expose_device_key.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn capabilities_require_components_with_typed_equality() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/capability_requires_component.rs");
    cases.compile_fail("tests/compile_fail/capability_requires_partial_eq.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn binding_authoring_requires_the_registered_driver_configuration() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/binding_authoring_rejects_wrong_configuration.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn continuous_flow_intervals_cannot_be_exchanged() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/continuous_flow_intervals_cannot_be_exchanged.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn role_presentation_cannot_be_constructed_outside_the_crate() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail(
        "tests/compile_fail/role_presentation_cannot_be_constructed_outside_the_crate.rs",
    );
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn apply_permit_cannot_be_constructed() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/apply_permit_cannot_be_constructed.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn apply_permit_cannot_be_matched() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/apply_permit_cannot_be_matched.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn attempt_and_session_authorities_enforce_one_owner() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/attempt_completion_rejects_wrong_configuration.rs");
    cases.compile_fail("tests/compile_fail/authorities_cannot_be_constructed.rs");
    cases.compile_fail("tests/compile_fail/authorities_cannot_be_cloned.rs");
    cases.compile_fail("tests/compile_fail/authorities_cannot_be_serialized.rs");
    cases.compile_fail("tests/compile_fail/attempt_completion_cannot_finish_twice.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn device_reporter_scan_is_unavailable_after_discovery_work_replaced_it() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/device_reporter_scan_is_unavailable.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn device_reporter_discover_cannot_receive_world() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/device_reporter_discover_cannot_receive_world.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn device_scan_unchanged_is_unavailable_after_scheduler_cadence_replaced_it() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/device_scan_unchanged_is_unavailable.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn runtime_discovery_limits_require_nonzero_values() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/runtime_discovery_limits_reject_zero.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn discovery_jobs_cannot_receive_world_or_capture_non_send_state_and_own_device_scans() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/discovery_job_cannot_receive_world.rs");
    cases.compile_fail("tests/compile_fail/discovery_job_cannot_capture_non_send_state.rs");
    cases.compile_fail("tests/compile_fail/discovery_job_requires_owned_device_scan.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn a_driver_cannot_assert_a_datum_arrival() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/driver_cannot_assert_a_datum_arrival.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn a_driver_ledger_does_not_yield_an_owned_authority() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/ledger_does_not_yield_an_owned_authority.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn driver_ledger_outcomes_must_be_used() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/ledger_outcomes_must_be_used.rs");
}
