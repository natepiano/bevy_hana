//! Reporter discovery work is scheduled by the kernel, not by the reporter.

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
