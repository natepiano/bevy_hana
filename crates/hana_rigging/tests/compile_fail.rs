//! Compile-time API boundaries that reporters and endpoint drivers must not bypass.

use bevy::app::App;
use bevy::tasks::IoTaskPool;
use hana_rigging::DeviceReporter;
use hana_rigging::DeviceScan;
use hana_rigging::DiscoveryCadence;
use hana_rigging::DiscoveryJob;
use hana_rigging::DiscoverySchedulerError;
use hana_rigging::DiscoverySchedulerState;
use hana_rigging::DiscoveryStatus;
use hana_rigging::DiscoveryStatusError;
use hana_rigging::DiscoveryWork;
use hana_rigging::LastDiscoveryOutcome;
use hana_rigging::MainThreadDiscoveryJob;
use hana_rigging::ReporterCoverage;
use hana_rigging::ReporterRegistration;
use hana_rigging::RiggingAppExt;
use hana_rigging::RiggingPlugin;
use hana_rigging::StartupDiscoveryState;

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
-> Result<(), DiscoveryStatusError> {
    assert!(IoTaskPool::try_get().is_none());
    let mut app = App::new();
    app.add_plugins(RiggingPlugin);
    let reporter = app.add_device_reporter(
        BackgroundReporter,
        ReporterRegistration::required(
            DiscoveryCadence::OnDemand,
            ReporterCoverage::MatchingEvidenceOnly,
        ),
    );

    app.update();

    let discovery_status = app.world().resource::<DiscoveryStatus>();
    assert_eq!(
        discovery_status.scheduler,
        DiscoverySchedulerState::Failed {
            error: DiscoverySchedulerError::IoTaskPoolUnavailable,
        }
    );
    assert!(matches!(
        discovery_status.startup,
        StartupDiscoveryState::Discovering
    ));
    let reporter_discovery_status = discovery_status.reporter_status(reporter)?;
    assert!(matches!(
        reporter_discovery_status.last_outcome,
        LastDiscoveryOutcome::NotCompleted
    ));
    assert_eq!(reporter_discovery_status.completed_batches, 0);

    Ok(())
}

#[test]
fn immediate_reporter_completes_without_io_pool_initialization() -> Result<(), DiscoveryStatusError>
{
    assert!(IoTaskPool::try_get().is_none());
    let mut app = App::new();
    app.add_plugins(RiggingPlugin);
    let reporter = app.add_device_reporter(
        ImmediateReporter,
        ReporterRegistration::required(
            DiscoveryCadence::OnDemand,
            ReporterCoverage::MatchingEvidenceOnly,
        ),
    );

    app.update();
    app.update();

    assert!(IoTaskPool::try_get().is_none());
    let reporter_discovery_status = app
        .world()
        .resource::<DiscoveryStatus>()
        .reporter_status(reporter)?;
    assert_eq!(reporter_discovery_status.completed_batches, 1);
    assert!(matches!(
        reporter_discovery_status.last_outcome,
        LastDiscoveryOutcome::Succeeded { .. }
    ));

    Ok(())
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn constructor_cannot_bypass_validated_constructors() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/constructor_bypass.rs");
}

#[test]
#[ignore = "CI-only compile-time API test"]
fn device_kind_requires_a_wildcard_arm() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/device_kind_requires_a_wildcard_arm.rs");
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
fn attempt_progress_finished_requires_an_attempt_outcome() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail(
        "tests/compile_fail/attempt_progress_finished_requires_an_attempt_outcome.rs",
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
fn lifecycle_mutation_and_request_minting_are_kernel_only() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/lifecycle_mutation_is_kernel_only.rs");
    cases.compile_fail("tests/compile_fail/state_issued_request_cannot_be_constructed.rs");
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
