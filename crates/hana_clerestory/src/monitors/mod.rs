mod current_monitor;
mod device_association;
mod display_product_name;
mod identity;
#[cfg(feature = "monitor-probe")]
mod monitor_probe;
mod topology;

use bevy::prelude::App;
use bevy::prelude::ApplyDeferred;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PreStartup;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Update;
use bevy::prelude::resource_changed;
use bevy::prelude::warn;
use bevy::winit::WinitMonitors;
pub use current_monitor::CurrentMonitor;
use current_monitor::clear_monitor_selection_inputs;
pub(crate) use current_monitor::current_monitor_from_association;
pub(crate) use current_monitor::exact_monitor_association;
pub(crate) use current_monitor::install_current_monitor_from_association;
pub(crate) use current_monitor::update_current_monitor;
pub use device_association::MonitorDeviceAssociation;
pub use device_association::MonitorDeviceKeyLookup;
pub(crate) use device_association::MonitorDeviceLookup;
pub use device_association::MonitorReportedHandleLookup;
pub use display_product_name::DisplayProductName;
use hana_rigging::prelude::AuthoritativeReporterCoverage;
use hana_rigging::prelude::CoveredDeviceIdentitySpace;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::DiscoveryCadence;
use hana_rigging::prelude::DiscoveryControl;
use hana_rigging::prelude::ReporterCoverage;
use hana_rigging::prelude::ReporterId;
use hana_rigging::prelude::ReporterRegistration;
use hana_rigging::prelude::RiggingAppExt;
use identity::MonitorConfiguration;
#[cfg(any(test, feature = "test"))]
pub(crate) use identity::PanelFingerprint;
pub(crate) use identity::PanelIdentity;
pub(crate) use identity::PanelIdentityEvidence;
pub use topology::CurrentMonitorIndex;
pub(crate) use topology::DisplayDeviceEvidence;
pub(crate) use topology::DisplayTopologyObservation;
pub(crate) use topology::EnumeratedDisplayEvidence;
#[cfg(test)]
pub(crate) use topology::InjectedMonitorEvidence;
#[cfg(test)]
pub(crate) use topology::InjectedWinitMonitorOrder;
pub use topology::LiveMonitor;
#[cfg(test)]
pub(crate) use topology::MonitorConnected;
pub use topology::MonitorDescriptor;
#[cfg(test)]
pub(crate) use topology::MonitorDisconnected;
pub use topology::MonitorTopologyRevision;
pub use topology::Monitors;
use topology::init_monitors;
use topology::update_monitors;

use crate::ClerestoryPreStartupSet;
use crate::ClerestoryUpdateSet;
use crate::Platform;
use crate::constants::MONITOR_DISCOVERY_BACKSTOP;
#[cfg(any(test, feature = "test"))]
use crate::display_test_adapter;
use crate::reporter;
#[cfg(any(test, feature = "test"))]
use crate::reporter::InjectedFreshWinitDisplays;
use crate::reporter::MonitorReporter;

/// Plugin that manages the `Monitors` resource.
pub(crate) struct MonitorPlugin;

impl Plugin for MonitorPlugin {
    fn build(&self, app: &mut App) {
        let configuration = MonitorConfiguration::register(*app.world().resource::<Platform>());
        if let Ok(edid_serial_scheme) = reporter::edid_serial_scheme() {
            app.register_device_scheme(edid_serial_scheme);
        }
        let monitor_reporter_id = app.add_device_reporter(
            MonitorReporter,
            ReporterRegistration::required(
                DiscoveryCadence::EventDriven {
                    backstop: MONITOR_DISCOVERY_BACKSTOP,
                },
                ReporterCoverage::EstablishesAbsence(AuthoritativeReporterCoverage::one(
                    CoveredDeviceIdentitySpace::AllKeysOfKind {
                        kind: DeviceKind::Display,
                    },
                )),
            ),
        );
        app.insert_resource(configuration)
            .insert_resource(MonitorReporterId(monitor_reporter_id))
            .init_resource::<MonitorDeviceAssociation>()
            .init_resource::<DisplayTopologyObservation>()
            .add_observer(clear_monitor_selection_inputs)
            .add_observer(install_current_monitor_from_association)
            .add_systems(
                PreStartup,
                refresh_monitor_device_association
                    .after(ClerestoryPreStartupSet::MonitorsInitialized),
            )
            .add_systems(
                PreStartup,
                init_monitors.in_set(ClerestoryPreStartupSet::MonitorsInitialized),
            )
            .add_systems(
                Update,
                (
                    update_monitors,
                    ApplyDeferred,
                    refresh_monitor_device_association,
                    mark_monitor_reporter_dirty.run_if(resource_changed::<MonitorTopologyRevision>),
                )
                    .chain()
                    .in_set(ClerestoryUpdateSet::MonitorTopology),
            )
            .add_systems(
                Update,
                (update_current_monitor, ApplyDeferred)
                    .chain()
                    .in_set(ClerestoryUpdateSet::CurrentMonitor),
            );
        #[cfg(any(test, feature = "test"))]
        app.init_resource::<InjectedFreshWinitDisplays>();
        #[cfg(any(test, feature = "test"))]
        display_test_adapter::install_scripted_display_topology(app);
    }

    fn finish(&self, app: &mut App) {
        assert!(
            app.world().contains_resource::<WinitMonitors>(),
            "hana_clerestory requires bevy's WinitPlugin: WinitMonitors is missing"
        );
    }
}

/// Keep the exact reporter-key bridge aligned with the installed monitor topology.
fn refresh_monitor_device_association(
    monitors: Res<Monitors>,
    mut association: ResMut<MonitorDeviceAssociation>,
) {
    association.refresh(&monitors);
}

/// Process-local handle the rigging kernel issued for [`MonitorReporter`].
///
/// The handle is kept so the display-configuration path can mark the reporter dirty. A reporter
/// that could not be named again would fall back to its one-second backstop, which is a delay a
/// user watching a monitor reconnect would see.
#[derive(Resource)]
pub(crate) struct MonitorReporterId(ReporterId);

#[cfg(any(test, feature = "test"))]
impl MonitorReporterId {
    /// Kernel handle the display reporter was registered under.
    pub(crate) const fn get(&self) -> ReporterId { self.0 }
}

/// Latest installed topology revision covered by a monitor discovery request.
///
/// The reporter's required registration requests its startup discovery before `init_monitors`
/// installs revision zero. `init_monitors` binds that request to the initial revision, and later
/// dirty requests advance this resource. Comparing revisions therefore distinguishes initial
/// topology installation from an operating-system topology notification without relying on Bevy
/// change ticks or relative system order.
#[derive(Resource)]
struct MonitorDiscoveryRequestCoverage(MonitorTopologyRevision);

impl MonitorDiscoveryRequestCoverage {
    const fn from_initial_topology(revision: MonitorTopologyRevision) -> Self { Self(revision) }

    fn covers(&self, revision: MonitorTopologyRevision) -> bool { self.0 == revision }

    const fn record_dirty_request(&mut self, revision: MonitorTopologyRevision) {
        self.0 = revision;
    }
}

/// Ask the kernel to re-enumerate displays as soon as the installed topology changes.
///
/// `MonitorTopologyRevision` advances only when the operating system's display-configuration path
/// produced a real change, so this is the notification the `DiscoveryCadence::EventDriven`
/// registration expects. Without it a reconnected panel would wait out the backstop.
fn mark_monitor_reporter_dirty(
    revision: Res<MonitorTopologyRevision>,
    monitor_reporter_id: Res<MonitorReporterId>,
    mut request_coverage: ResMut<MonitorDiscoveryRequestCoverage>,
    mut discovery_control: ResMut<DiscoveryControl>,
) {
    if request_coverage.covers(*revision) {
        return;
    }
    match discovery_control.mark_dirty(monitor_reporter_id.0) {
        Ok(()) => request_coverage.record_dirty_request(*revision),
        Err(error) => {
            warn!("[mark_monitor_reporter_dirty] kernel refused the notification: {error}");
        },
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use bevy::app::TaskPoolPlugin;
    use bevy::asset::AssetPlugin;
    #[cfg(feature = "monitor-probe")]
    use bevy::diagnostic::FrameCount;
    use bevy::prelude::IVec2;
    use bevy::prelude::On;
    use bevy::window::Monitor;
    use bevy::winit::WinitPlugin;
    use hana_rigging::prelude::CompletedDiscoveryOutcome;
    use hana_rigging::prelude::DiscoveryFinished;
    use hana_rigging::prelude::DiscoveryStatus;
    use hana_rigging::prelude::ReporterActivity;
    use hana_rigging::prelude::RiggingPlugin;
    use hana_rigging::prelude::RiggingSystems;
    use hana_rigging::prelude::StartupDiscoveryChanged;
    use hana_rigging::prelude::StartupDiscoveryState;
    use topology::InjectedMonitorEvidence;
    use topology::InjectedWinitMonitorOrder;

    use super::*;

    const IDLE_UPDATES: usize = 3;
    const TOPOLOGY_CHANGE_UPDATES: usize = 4;

    #[derive(Clone, Copy, Debug)]
    enum ReporterCollectionOrder {
        TopologyBeforeCollection,
        CollectionBeforeTopology,
    }

    #[derive(Clone, Copy)]
    enum WinitPluginOrder {
        MonitorFirst,
        WinitFirst,
    }

    const WINIT_ON_ANY_THREAD: WinitPlugin = WinitPlugin {
        run_on_any_thread: true,
    };

    fn finish_with_real_winit_plugin(order: WinitPluginOrder) {
        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
            .insert_resource(Platform::X11);
        match order {
            WinitPluginOrder::MonitorFirst => {
                app.add_plugins(MonitorPlugin)
                    .add_plugins(WINIT_ON_ANY_THREAD);
            },
            WinitPluginOrder::WinitFirst => {
                app.add_plugins(WINIT_ON_ANY_THREAD)
                    .add_plugins(MonitorPlugin);
            },
        }
        app.finish();
    }

    #[cfg(target_os = "macos")]
    mod macos_main_thread_finish {
        use std::panic::AssertUnwindSafe;
        use std::panic::catch_unwind;
        use std::sync::OnceLock;

        use super::WinitPluginOrder;
        use super::finish_with_real_winit_plugin;

        static RESULT: OnceLock<Result<(), String>> = OnceLock::new();

        extern "C" fn run_selected_finish_contract() {
            let selected = std::env::args().find_map(|argument| {
                [
                    (
                        "monitor_plugin_before_real_winit_plugin_finishes",
                        WinitPluginOrder::MonitorFirst,
                    ),
                    (
                        "real_winit_plugin_before_monitor_plugin_finishes",
                        WinitPluginOrder::WinitFirst,
                    ),
                ]
                .into_iter()
                .find_map(|(test_name, order)| argument.contains(test_name).then_some(order))
            });
            let Some(selected) = selected else {
                return;
            };
            let result = catch_unwind(AssertUnwindSafe(|| {
                finish_with_real_winit_plugin(selected);
            }))
            .map_err(|panic_payload| {
                panic_payload.downcast_ref::<&str>().map_or_else(
                    || String::from("real WinitPlugin finish path panicked"),
                    |message| (*message).to_owned(),
                )
            });
            let _ = RESULT.set(result);
        }

        // The Rust test harness invokes each test on a worker thread, while winit requires event
        // loop construction on the macOS process main thread. Nextest runs each unit test in its
        // own process, so this initializer executes only the selected real-Winit contract before
        // the harness moves control to its worker.
        #[used]
        #[unsafe(link_section = "__DATA,__mod_init_func")]
        static MAIN_THREAD_FINISH_CONTRACT: extern "C" fn() = run_selected_finish_contract;

        pub(super) fn assert_succeeded() {
            match RESULT.get() {
                Some(Ok(())) => {},
                Some(Err(message)) => panic!("{message}"),
                None => panic!("macOS main-thread WinitPlugin contract did not run"),
            }
        }
    }

    #[test]
    #[cfg_attr(
        not(target_os = "macos"),
        ignore = "builds a real winit event loop, which needs a display server"
    )]
    fn monitor_plugin_before_real_winit_plugin_finishes() {
        #[cfg(target_os = "macos")]
        macos_main_thread_finish::assert_succeeded();
        #[cfg(not(target_os = "macos"))]
        finish_with_real_winit_plugin(WinitPluginOrder::MonitorFirst);
    }

    #[test]
    #[cfg_attr(
        not(target_os = "macos"),
        ignore = "builds a real winit event loop, which needs a display server"
    )]
    fn real_winit_plugin_before_monitor_plugin_finishes() {
        #[cfg(target_os = "macos")]
        macos_main_thread_finish::assert_succeeded();
        #[cfg(not(target_os = "macos"))]
        finish_with_real_winit_plugin(WinitPluginOrder::WinitFirst);
    }

    #[test]
    #[should_panic(
        expected = "hana_clerestory requires bevy's WinitPlugin: WinitMonitors is missing"
    )]
    fn omitting_winit_plugin_fails_with_named_resource_message() {
        let mut app = App::new();
        app.insert_resource(Platform::X11)
            .add_plugins(MonitorPlugin);

        app.finish();
    }

    #[derive(Default, Resource)]
    struct DiscoveryLifecycleObservations {
        finished: Vec<ReporterId>,
        startup:  Vec<StartupDiscoveryState>,
    }

    fn observe_discovery_finished(
        event: On<DiscoveryFinished>,
        mut observations: ResMut<DiscoveryLifecycleObservations>,
    ) {
        assert!(matches!(
            &event.outcome,
            CompletedDiscoveryOutcome::Succeeded { .. }
        ));
        observations.finished.push(event.reporter);
    }

    fn observe_startup_discovery_changed(
        event: On<StartupDiscoveryChanged>,
        mut observations: ResMut<DiscoveryLifecycleObservations>,
    ) {
        observations.startup.push(event.state.clone());
    }

    fn monitor_reporter_app(reporter_collection_order: ReporterCollectionOrder) -> App {
        let mut app = App::new();
        app.insert_resource(Platform::X11)
            .insert_resource(WinitMonitors::default())
            .init_resource::<InjectedMonitorEvidence>()
            .init_resource::<InjectedWinitMonitorOrder>()
            .init_resource::<DiscoveryLifecycleObservations>()
            .add_observer(observe_discovery_finished)
            .add_observer(observe_startup_discovery_changed);
        #[cfg(feature = "monitor-probe")]
        app.init_resource::<FrameCount>();

        match reporter_collection_order {
            ReporterCollectionOrder::TopologyBeforeCollection => {
                app.add_plugins(MonitorPlugin)
                    .add_plugins(RiggingPlugin)
                    .configure_sets(
                        Update,
                        ClerestoryUpdateSet::MonitorTopology.before(RiggingSystems::Collect),
                    );
            },
            ReporterCollectionOrder::CollectionBeforeTopology => {
                app.add_plugins(RiggingPlugin)
                    .add_plugins(MonitorPlugin)
                    .configure_sets(
                        Update,
                        ClerestoryUpdateSet::MonitorTopology.after(RiggingSystems::Collect),
                    );
            },
        }
        app
    }

    fn completed_batches(app: &App, reporter_id: ReporterId) -> u64 {
        let reporter_status = app
            .world()
            .resource::<DiscoveryStatus>()
            .reporter_status(reporter_id);
        assert!(reporter_status.is_ok());
        reporter_status
            .ok()
            .map_or(0, |reporter_status| reporter_status.completed_batches)
    }

    fn assert_reporter_lifecycle(reporter_collection_order: ReporterCollectionOrder) {
        let mut app = monitor_reporter_app(reporter_collection_order);
        let reporter_id = app.world().resource::<MonitorReporterId>().0;
        assert!(matches!(
            app.world().resource::<DiscoveryStatus>().startup,
            StartupDiscoveryState::Discovering
        ));

        app.update();
        assert!(matches!(
            app.world()
                .resource::<DiscoveryStatus>()
                .reporter_status(reporter_id),
            Ok(reporter_status)
                if matches!(&reporter_status.activity, ReporterActivity::Queued { .. })
        ));
        app.update();

        assert!(matches!(
            app.world().resource::<DiscoveryStatus>().startup,
            StartupDiscoveryState::Ready
        ));
        assert_eq!(completed_batches(&app, reporter_id), 1);
        {
            let observations = app.world().resource::<DiscoveryLifecycleObservations>();
            assert_eq!(observations.finished, [reporter_id]);
            assert_eq!(observations.startup, [StartupDiscoveryState::Ready]);
        }

        for _ in 0..IDLE_UPDATES {
            app.update();
        }
        assert_eq!(completed_batches(&app, reporter_id), 1);
        {
            let observations = app.world().resource::<DiscoveryLifecycleObservations>();
            assert_eq!(observations.finished, [reporter_id]);
            assert_eq!(observations.startup, [StartupDiscoveryState::Ready]);
        }

        app.world_mut().spawn(Monitor {
            name:                    None,
            physical_height:         1_080,
            physical_width:          1_920,
            physical_position:       IVec2::ZERO,
            refresh_rate_millihertz: None,
            scale_factor:            1.0,
            video_modes:             Vec::new(),
        });
        for _ in 0..TOPOLOGY_CHANGE_UPDATES {
            app.update();
        }

        assert_eq!(
            *app.world().resource::<MonitorTopologyRevision>(),
            MonitorTopologyRevision::from_test_raw(1)
        );
        assert_eq!(completed_batches(&app, reporter_id), 2);
        {
            let observations = app.world().resource::<DiscoveryLifecycleObservations>();
            assert_eq!(observations.finished, [reporter_id, reporter_id]);
            assert_eq!(observations.startup, [StartupDiscoveryState::Ready]);
        }

        for _ in 0..IDLE_UPDATES {
            app.update();
        }
        assert_eq!(completed_batches(&app, reporter_id), 2);
        let observations = app.world().resource::<DiscoveryLifecycleObservations>();
        assert_eq!(observations.finished, [reporter_id, reporter_id]);
        assert_eq!(observations.startup, [StartupDiscoveryState::Ready]);
    }

    #[test]
    fn startup_and_topology_change_each_request_one_discovery_under_both_system_orders() {
        assert_reporter_lifecycle(ReporterCollectionOrder::TopologyBeforeCollection);
        assert_reporter_lifecycle(ReporterCollectionOrder::CollectionBeforeTopology);
    }
}
