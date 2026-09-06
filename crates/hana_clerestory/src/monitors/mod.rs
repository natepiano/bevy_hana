mod current_monitor;
mod display_product_name;
mod identity;
mod live_display_endpoint;
#[cfg(feature = "monitor-probe")]
mod monitor_probe;
mod topology;

use bevy::ecs::reflect::ReflectResource;
use bevy::prelude::App;
use bevy::prelude::ApplyDeferred;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PreStartup;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Update;
use bevy::prelude::resource_changed;
use bevy::prelude::warn;
use bevy::winit::WinitMonitors;
pub use current_monitor::CurrentMonitor;
pub(crate) use current_monitor::CurrentMonitorEntity;
use current_monitor::clear_monitor_selection_inputs;
pub(crate) use current_monitor::current_monitor_from_association;
pub(crate) use current_monitor::exact_monitor_association;
pub(crate) use current_monitor::install_current_monitor_from_association;
pub(crate) use current_monitor::update_current_monitor;
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
pub use identity::DisplayFingerprint;
pub use identity::DisplayIdentity;
pub(crate) use identity::DisplayIdentityEvidence;
use identity::MonitorConfiguration;
pub use live_display_endpoint::LiveDisplayContradictionCount;
pub use live_display_endpoint::LiveDisplayDevices;
pub use live_display_endpoint::LiveDisplayEndpoint;
pub use live_display_endpoint::LiveDisplayEndpointLookup;
pub use live_display_endpoint::LiveDisplayMatchError;
pub use live_display_endpoint::LiveDisplayMonitor;
pub use topology::CurrentMonitorIndex;
pub(crate) use topology::DisplayDeviceEvidence;
pub(crate) use topology::DisplayTopologyObservation;
pub(crate) use topology::EnumeratedDisplayEvidence;
#[cfg(any(test, feature = "test"))]
pub(crate) use topology::InjectedMonitorEvidence;
#[cfg(any(test, feature = "test"))]
pub(crate) use topology::InjectedWinitMonitorOrder;
pub use topology::LiveMonitor;
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
#[cfg(any(test, feature = "test"))]
use crate::display_test_adapter::DisplayTestAdapterInstallation;
use crate::reporter;
use crate::reporter::MonitorReporter;

/// Plugin that manages the `Monitors` resource.
pub(crate) struct MonitorPlugin;

impl Plugin for MonitorPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(any(test, feature = "test"))]
        let display_enumeration_source = display_enumeration_source_for_test_adapter_installation(
            display_test_adapter::DisplayTestAdapter::installation(app.world()),
        );
        #[cfg(not(any(test, feature = "test")))]
        let display_enumeration_source = DisplayEnumerationSource::LiveWinit;
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
                std::time::Duration::from_secs(10),
            ),
        );
        app.insert_resource(configuration)
            .insert_resource(display_enumeration_source)
            .insert_resource(MonitorReporterId::new(monitor_reporter_id))
            .init_resource::<DisplayTopologyObservation>()
            .add_observer(clear_monitor_selection_inputs)
            .add_observer(install_current_monitor_from_association)
            .add_systems(
                PreStartup,
                init_monitors.in_set(ClerestoryPreStartupSet::MonitorsInitialized),
            )
            .add_systems(
                Update,
                (
                    update_monitors,
                    ApplyDeferred,
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
        display_test_adapter::install_scripted_display_topology(app);
    }

    fn finish(&self, app: &mut App) {
        assert!(
            app.world().contains_resource::<WinitMonitors>(),
            "hana_clerestory requires bevy's WinitPlugin: WinitMonitors is missing"
        );
    }
}

/// Compile-time witness that production display enumeration selects live winit.
#[cfg(not(test))]
#[doc(hidden)]
pub struct LiveWinitProductionBackendSelection;

/// Display enumeration source selected when `MonitorPlugin` is installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Resource)]
#[reflect(Resource)]
#[type_path = "hana_clerestory::monitors"]
pub enum DisplayEnumerationSource {
    /// The production reporter reads the current display list from winit.
    LiveWinit,
    /// A caller explicitly installed `DisplayTestAdapter`.
    ExplicitTestAdapter,
    /// The injected display resource exists without an explicitly installed adapter.
    InjectedWithoutExplicitAdapter,
    /// The adapter resource exists without its injected display resource.
    ExplicitAdapterWithoutInjectedSource,
}

#[cfg(any(test, feature = "test"))]
const fn display_enumeration_source_for_test_adapter_installation(
    display_test_adapter_installation: DisplayTestAdapterInstallation,
) -> DisplayEnumerationSource {
    match display_test_adapter_installation {
        DisplayTestAdapterInstallation::ExplicitlyInstalled => {
            DisplayEnumerationSource::ExplicitTestAdapter
        },
        DisplayTestAdapterInstallation::NotInstalled => DisplayEnumerationSource::LiveWinit,
        DisplayTestAdapterInstallation::InjectedWithoutExplicitAdapter => {
            DisplayEnumerationSource::InjectedWithoutExplicitAdapter
        },
        DisplayTestAdapterInstallation::ExplicitAdapterWithoutInjectedSource => {
            DisplayEnumerationSource::ExplicitAdapterWithoutInjectedSource
        },
    }
}

#[cfg(all(feature = "test", not(test)))]
const _: () = assert!(matches!(
    display_enumeration_source_for_test_adapter_installation(
        display_test_adapter::DisplayTestAdapterInstallation::NotInstalled,
    ),
    DisplayEnumerationSource::LiveWinit,
));

/// Process-local handle the rigging kernel issued for [`MonitorReporter`].
///
/// The handle is kept so the display-configuration path can mark the reporter dirty. A reporter
/// that could not be named again would fall back to its one-second backstop, which is a delay a
/// user watching a monitor reconnect would see.
#[derive(Resource)]
pub(crate) struct MonitorReporterId(ReporterId);

impl MonitorReporterId {
    /// Name the reporter the window driver resolves its display capability under.
    ///
    /// `MonitorPlugin` inserts this resource for the reporter it registers, which is the only
    /// producer in production. A test that drives the driver under a reporter of its own —
    /// the conformance walk's scripted reporter — overrides the resource with this, so
    /// `resolve_target`, which reads it at call time, resolves the capability the walk publishes
    /// rather than the one the shipped monitor reporter publishes beside it.
    pub(crate) const fn new(reporter: ReporterId) -> Self { Self(reporter) }

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
/// registration relies on. Without it a reconnected display would wait out the backstop.
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
    use hana_rigging::prelude::FirstCompleteSetStatus;
    use hana_rigging::prelude::ReporterActivityView;
    use hana_rigging::prelude::ReporterHealth;
    use hana_rigging::prelude::RiggingPlugin;
    use hana_rigging::prelude::RiggingSystems;
    use hana_rigging::prelude::StartupDiscoveryChanged;
    use hana_rigging::prelude::StartupDiscoveryState;
    use topology::InjectedMonitorEvidence;
    use topology::InjectedWinitMonitorOrder;

    use super::*;
    use crate::reporter::InjectedFreshWinitDisplays;

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
                None => panic!(
                    "the main-thread WinitPlugin contract did not run. It is started by a \
                     `__mod_init_func` initializer that looks for this test's name in the \
                     process arguments, so it only runs under a runner that gives each test \
                     its own process. Use `cargo nextest run`, which is what CI uses; a plain \
                     `cargo test` over the whole crate reaches here instead"
                ),
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
            .init_resource::<InjectedFreshWinitDisplays>()
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
        reporter_health(app, reporter_id).map_or(0, ReporterHealth::completed_runs)
    }

    fn reporter_health(app: &App, reporter_id: ReporterId) -> Option<&ReporterHealth> {
        app.world()
            .iter_entities()
            .filter_map(|entity| entity.get::<ReporterHealth>())
            .find(|health| health.belongs_to(reporter_id))
    }

    fn assert_reporter_lifecycle(reporter_collection_order: ReporterCollectionOrder) {
        let mut app = monitor_reporter_app(reporter_collection_order);
        let reporter_id = app.world().resource::<MonitorReporterId>().0;
        assert!(matches!(
            reporter_health(&app, reporter_id).map(ReporterHealth::first_complete_set),
            Some(FirstCompleteSetStatus::Waiting(_))
        ));

        app.update();
        assert!(matches!(
            reporter_health(&app, reporter_id).map(ReporterHealth::activity),
            Some(ReporterActivityView::Queued { .. })
        ));
        app.update();

        assert!(matches!(
            reporter_health(&app, reporter_id).map(ReporterHealth::first_complete_set),
            Some(FirstCompleteSetStatus::Completed { .. })
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

        let monitor_entity = app
            .world_mut()
            .spawn(Monitor {
                name:                    None,
                physical_height:         1_080,
                physical_width:          1_920,
                physical_position:       IVec2::ZERO,
                refresh_rate_millihertz: None,
                scale_factor:            1.0,
                video_modes:             Vec::new(),
            })
            .id();
        app.world_mut()
            .resource_mut::<InjectedFreshWinitDisplays>()
            .entities
            .push(monitor_entity);
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
