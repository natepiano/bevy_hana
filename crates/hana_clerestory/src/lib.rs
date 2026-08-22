#![doc = include_str!("../README.md")]
//!
//! # Technical Details
//!
//! ## The Problem
//!
//! On macOS with multiple monitors that have different scale factors (e.g., a Retina display
//! at scale 2.0 and an external monitor at scale 1.0), Bevy's window positioning has issues:
//!
//! 1. **`Window.position` is unreliable at startup**: When a window is created, `Window.position`
//!    is `Automatic` (not `At(position)`), even though winit has placed the window at a specific
//!    physical position.
//!
//! 2. **Scale factor conversion in `changed_windows`**: When you modify `Window.resolution`, Bevy's
//!    `changed_windows` system applies scale factor conversion if `scale_factor !=
//!    cached_scale_factor`. This corrupts the size when moving windows between monitors with
//!    different scale factors.
//!
//! 3. **Timing of scale factor updates**: The `CachedWindow` is updated after winit events are
//!    processed, but our systems run before we receive the `ScaleFactorChanged` event.
//!
//! ## The Solution
//!
//! This plugin uses winit directly to capture the actual window position at startup,
//! compensates for scale factor conversions, restores `Window.position`,
//! `Window.resolution`, visibility, and monitor state across monitors.
//!
//! The plugin automatically hides the window during startup and shows it after positioning
//! is complete, preventing any visual flash at the default position.
//!
//! See the `custom_app_name` example for how to override the `app_name` used in the path
//! (default is to choose the executable name).
//!
//! See the `custom_path` example for how to override the full path to the state file.

mod constants;
#[cfg(any(test, feature = "test"))]
mod display_test_adapter;
mod driver;
mod events;
#[cfg(target_os = "macos")]
mod macos_tabbing_fix;
mod managed;
mod monitors;
mod persistence;
mod platform;
mod recovery;
mod reporter;
mod resize_announcement;
mod restore;
mod restore_window_config;
mod visibility;
#[cfg(all(target_os = "windows", feature = "workaround-winit-4341"))]
mod windows_dpi_fix;

use std::path::PathBuf;

use bevy::camera::CameraUpdateSystems;
use bevy::prelude::App;
#[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
use bevy::prelude::ApplyDeferred;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::PreStartup;
#[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
use bevy::prelude::Res;
#[cfg(all(target_os = "windows", feature = "workaround-winit-4341"))]
use bevy::prelude::Startup;
use bevy::prelude::SystemSet;
use bevy::prelude::Update;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::debug;
use bevy::window::PrimaryWindow;
#[cfg(any(test, feature = "test"))]
pub use display_test_adapter::DisplayTestAdapter;
#[cfg(any(test, feature = "test"))]
pub use display_test_adapter::DisplayTestDescriptor;
#[cfg(any(test, feature = "test"))]
pub use display_test_adapter::DisplayTestDeviceKey;
#[cfg(any(test, feature = "test"))]
pub use display_test_adapter::DisplayTestEnumeration;
#[cfg(any(test, feature = "test"))]
pub use display_test_adapter::DisplayTestReporterLookup;
use driver::WindowDriverAttemptResults;
use driver::WindowDriverId;
use driver::WindowEndpointDriver;
pub use events::ExpectedLogicalPosition;
pub use events::ExpectedPhysicalPosition;
pub use events::ObservedLogicalPosition;
pub use events::ObservedPhysicalPosition;
pub use events::WindowRestoreMismatch;
pub use events::WindowRestored;
use hana_rigging::prelude::RiggingAppExt;
use hana_rigging::prelude::RiggingPlugin;
pub use managed::ManagedWindow;
pub use managed::ManagedWindowPersistence;
pub use managed::ManagedWindowReapplyOnRequest;
use managed::ManagedWindowRegistry;
use managed::author_window_bindings;
use managed::forget_stale_window_captures;
use managed::on_managed_window_added;
use managed::on_managed_window_load;
use managed::on_managed_window_removed;
use managed::on_primary_window_removed;
use managed::rebind_window_to_its_current_display;
use managed::synchronize_registered_window_binding_projections;
pub use monitors::CurrentMonitor;
pub use monitors::CurrentMonitorIndex;
pub use monitors::DisplayProductName;
pub use monitors::LiveMonitor;
pub use monitors::MonitorDescriptor;
pub use monitors::MonitorDeviceAssociation;
pub use monitors::MonitorDeviceKeyLookup;
use monitors::MonitorPlugin;
pub use monitors::MonitorReportedHandleLookup;
pub use monitors::MonitorTopologyRevision;
pub use monitors::Monitors;
use persistence::PersistencePlugin;
pub use platform::Platform;
use recovery::RecoveryPlugin;
use restore::RestorePlugin;
use restore_window_config::RestoreWindowConfig;

#[derive(Clone, Debug, Hash, PartialEq, Eq, SystemSet)]
enum ClerestoryPreStartupSet {
    MonitorsInitialized,
    PersistenceLoaded,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, SystemSet)]
pub(crate) enum ClerestoryUpdateSet {
    MonitorTopology,
    RecoveryTopology,
    CurrentMonitor,
    RecoveryWindow,
    Persistence,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, SystemSet)]
enum ClerestoryWindowDriverSet {
    BindingAuthoring,
    TargetPreparation,
}

/// The main plugin. See module docs for usage.
///
/// Default state file locations:
/// - macOS: `~/Library/Application Support/<executable_name>/windows.ron`
/// - Linux: `~/.config/<executable_name>/windows.ron`
/// - Windows: `C:\Users\<User>\AppData\Roaming\<executable_name>\windows.ron`
///
/// Unit struct version for convenience using `.add_plugins(WindowManagerPlugin)`.
pub struct WindowManagerPlugin;

impl WindowManagerPlugin {
    /// Create a plugin with a custom app name.
    ///
    /// Uses `config_dir()/<app_name>/windows.ron`.
    ///
    /// # Panics
    ///
    /// Panics if the config directory cannot be determined.
    #[must_use]
    #[expect(clippy::expect_used, reason = "fail fast if path cannot be determined")]
    pub fn with_app_name(app_name: impl Into<String>) -> impl Plugin {
        WindowManagerPluginCustomPath {
            path:                       persistence::get_state_path_for_app(&app_name.into())
                .expect("Could not determine state file path"),
            managed_window_persistence: ManagedWindowPersistence::default(),
        }
    }

    /// Create a plugin with a custom state file path.
    #[must_use]
    pub fn with_path(path: impl Into<PathBuf>) -> impl Plugin {
        WindowManagerPluginCustomPath {
            path:                       path.into(),
            managed_window_persistence: ManagedWindowPersistence::default(),
        }
    }

    /// Create a plugin with a specific persistence behavior.
    ///
    /// # Panics
    ///
    /// Panics if the config directory cannot be determined.
    #[must_use]
    #[expect(clippy::expect_used, reason = "fail fast if path cannot be determined")]
    pub fn with_persistence(managed_window_persistence: ManagedWindowPersistence) -> impl Plugin {
        WindowManagerPluginCustomPath {
            path: persistence::get_default_state_path()
                .expect("Could not determine state file path"),
            managed_window_persistence,
        }
    }
}

impl Plugin for WindowManagerPlugin {
    #[expect(clippy::expect_used, reason = "fail fast if path cannot be determined")]
    fn build(&self, app: &mut App) {
        app.add_plugins(WindowManagerPluginCustomPath {
            path:                       persistence::get_default_state_path()
                .expect("Could not determine state file path"),
            managed_window_persistence: ManagedWindowPersistence::default(),
        });
    }
}

/// Plugin variant with a custom state file path.
struct WindowManagerPluginCustomPath {
    path:                       PathBuf,
    managed_window_persistence: ManagedWindowPersistence,
}

impl Plugin for WindowManagerPluginCustomPath {
    fn build(&self, app: &mut App) {
        let path = self.path.clone();
        let managed_window_persistence = self.managed_window_persistence.clone();

        if !app.is_plugin_added::<RiggingPlugin>() {
            app.add_plugins(RiggingPlugin);
        }

        let window_driver = app.add_endpoint_driver(WindowEndpointDriver);

        let platform = Platform::detect();
        app.insert_resource(platform);

        hide_startup_window(app, platform);

        #[cfg(target_os = "macos")]
        {
            // App-wide opt-out of automatic window tabbing, before winit creates
            // any OS window. See `macos_tabbing_fix` module docs.
            macos_tabbing_fix::disable_automatic_tabbing();
            app.add_systems(
                Update,
                macos_tabbing_fix::disable_tabbing_on_managed.before(restore::restore_windows),
            );
        }

        #[cfg(all(target_os = "windows", feature = "workaround-winit-4341"))]
        {
            app.add_systems(Startup, windows_dpi_fix::install_dpi_fix);
            app.add_systems(Update, windows_dpi_fix::install_dpi_fix_on_managed);
        }

        app.configure_sets(
            PreStartup,
            (
                ClerestoryPreStartupSet::MonitorsInitialized,
                ClerestoryPreStartupSet::PersistenceLoaded,
            )
                .chain(),
        )
        .configure_sets(
            Update,
            (
                ClerestoryUpdateSet::MonitorTopology,
                ClerestoryUpdateSet::RecoveryTopology,
                ClerestoryUpdateSet::CurrentMonitor,
                ClerestoryUpdateSet::RecoveryWindow,
                ClerestoryUpdateSet::Persistence,
            )
                .chain(),
        )
        .add_plugins(MonitorPlugin)
        .add_plugins(RecoveryPlugin)
        .add_plugins(PersistencePlugin)
        .add_plugins(RestorePlugin)
        .insert_resource(WindowDriverId(window_driver))
        .init_resource::<WindowDriverAttemptResults>()
        .insert_resource(RestoreWindowConfig { path })
        .insert_resource(managed_window_persistence)
        .init_resource::<ManagedWindowRegistry>()
        .add_observer(on_managed_window_added)
        .add_observer(on_managed_window_removed)
        .add_observer(on_primary_window_removed)
        .add_observer(on_managed_window_load);

        app.add_systems(
            Update,
            (
                synchronize_registered_window_binding_projections,
                author_window_bindings,
                forget_stale_window_captures,
            )
                .chain()
                .in_set(ClerestoryWindowDriverSet::BindingAuthoring),
        );
        app.add_observer(rebind_window_to_its_current_display);

        // Chained so the frame that reveals a stranded window also takes the first observation of
        // where it landed; the baseline is closed out on the frame after that.
        app.add_systems(
            Update,
            (
                visibility::reveal_window_without_exact_display,
                managed::adopt_live_display_for_stranded_window,
            )
                .chain()
                .after(ClerestoryWindowDriverSet::BindingAuthoring),
        );

        // Announce the resizes this crate performs before any camera reads the new resolution.
        // See `resize_announcement` module docs.
        app.add_systems(
            PostUpdate,
            resize_announcement::announce_unpublished_resizes.before(CameraUpdateSystems),
        );

        // X11 frame extent compensation (W6 workaround, winit #4445).
        #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
        app.add_systems(
            Update,
            (
                restore::compensate_target_position
                    .after(restore::prepare_driver_restore_targets)
                    .before(restore::restore_windows)
                    .in_set(ClerestoryWindowDriverSet::TargetPreparation),
                ApplyDeferred
                    .after(restore::compensate_target_position)
                    .in_set(ClerestoryWindowDriverSet::TargetPreparation),
                // Re-apply the compensated position once the window is mapped: bevy 0.19
                // can ignore the first `set_outer_position` request while the X11 window is
                // unmapped, while a mapped window's `Window.position` readback matches the
                // requested compensated position plus `X11FrameTop`.
                restore::reapply_compensated_position
                    .after(restore::restore_windows)
                    .before(restore::check_restore_settling)
                    .in_set(ClerestoryWindowDriverSet::TargetPreparation),
            )
                .run_if(|p: Res<Platform>| p.is_x11()),
        );
    }
}

/// Hide an existing primary immediately; the observer handles a later `PrimaryWindow`.
/// X11 frame compensation keeps the window mapped so `_NET_FRAME_EXTENTS` is readable.
fn hide_startup_window(app: &mut App, platform: Platform) {
    if platform.should_hide_on_startup() {
        let mut query = app
            .world_mut()
            .query_filtered::<&mut Window, With<PrimaryWindow>>();
        if let Some(mut window) = query.iter_mut(app.world_mut()).next() {
            debug!("[build] Window already exists, hiding immediately");
            window.visible = false;
        } else {
            debug!("[build] Window doesn't exist yet, registering observer");
            app.add_observer(visibility::hide_window_on_creation);
        }
    } else {
        debug!("[build] Linux X11: skipping window hide for frame extent compensation");
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use bevy::MinimalPlugins;
    use bevy::prelude::Component;
    use bevy::prelude::Entity;
    use bevy::prelude::IVec2;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectComponent;
    use bevy::prelude::UVec2;
    use bevy::prelude::WindowPosition;
    use bevy::prelude::World;
    use bevy::prelude::default;
    use bevy::window::Monitor;
    use bevy::window::OnMonitor;
    use bevy::window::WindowPlugin;
    use bevy::winit::WinitMonitors;
    use hana_rigging::prelude::ApplyDeadline;
    use hana_rigging::prelude::ApplyPermit;
    use hana_rigging::prelude::AttemptId;
    use hana_rigging::prelude::AttemptLookup;
    use hana_rigging::prelude::AttemptOutcome;
    use hana_rigging::prelude::AttemptProgress;
    use hana_rigging::prelude::Attempts;
    use hana_rigging::prelude::AuthoredId;
    use hana_rigging::prelude::Binding;
    use hana_rigging::prelude::Bindings;
    use hana_rigging::prelude::CaptureOutcome;
    use hana_rigging::prelude::DeviceEndpoint;
    use hana_rigging::prelude::DeviceIdSource;
    use hana_rigging::prelude::DeviceKey;
    use hana_rigging::prelude::DeviceKind;
    use hana_rigging::prelude::Digest;
    use hana_rigging::prelude::EndpointDriver;
    use hana_rigging::prelude::EndpointId;
    use hana_rigging::prelude::HardwareInventory;
    use hana_rigging::prelude::LastKnownGoodConfiguration;
    use hana_rigging::prelude::OnAbort;
    use hana_rigging::prelude::OnSessionLoss;
    use hana_rigging::prelude::RecoveryPolicy;
    use hana_rigging::prelude::RequestedConfiguration;
    use hana_rigging::prelude::RetryOn;
    use hana_rigging::prelude::RiggingAppExt;
    use hana_rigging::prelude::RoleKey;
    use hana_rigging::prelude::RoleState;
    use monitors::PanelFingerprint;
    use persistence::EstablishedWindowPlacement;
    use persistence::PersistedPanelFingerprintV4;
    use persistence::PersistedPanelIdentityV4;
    use persistence::PersistedWindowStateDecodeOutcome;
    use persistence::PersistedWindowTargetV5;
    use reporter::InjectedFreshWinitDisplays;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::*;
    use crate::restore::InjectedWinitWindows;
    use crate::restore::RestorePreparation;
    use crate::restore::TargetPosition;
    use crate::restore::WindowApplyConfiguration;

    const IDENTIFIED_PANEL_EVIDENCE: &[u8] = b"phase-18-startup-panel";
    const SECOND_IDENTIFIED_PANEL_EVIDENCE: &[u8] = b"phase-18-startup-panel-two";
    const STARTUP_RESTORE_UPDATE_LIMIT: usize = 8;

    #[derive(Clone, Component, Debug, PartialEq, Eq, Reflect)]
    #[reflect(Component, PartialEq)]
    struct KernelRetirementConfiguration;

    struct KernelRetirementDriver;

    impl EndpointDriver for KernelRetirementDriver {
        type Configuration = KernelRetirementConfiguration;

        fn capture(
            &mut self,
            _: &mut World,
            _: &DeviceEndpoint,
        ) -> CaptureOutcome<Self::Configuration> {
            CaptureOutcome::Read(KernelRetirementConfiguration)
        }

        fn start_apply(
            &mut self,
            _: &mut World,
            _: &DeviceEndpoint,
            _: &Self::Configuration,
            _: AttemptId,
            _: ApplyPermit,
        ) {
        }

        fn poll(&mut self, _: &mut World, _: AttemptId) -> AttemptProgress {
            AttemptProgress::Finished(AttemptOutcome::Succeeded)
        }
    }

    struct ProductionPluginHarness {
        app:       App,
        directory: TempDir,
    }

    enum KernelInstallation {
        Preinstalled,
        WindowManagerOwned,
    }

    enum ComponentArrival {
        AssociationBeforeRegistration,
        RegistrationBeforeAssociation,
    }

    struct StartedProductionWindowApply {
        primary_window: Entity,
        role:           RoleKey,
        attempt_id:     AttemptId,
    }

    impl ProductionPluginHarness {
        fn new(kernel_installation: KernelInstallation) -> Result<Self, String> {
            Self::with_window_plugin(kernel_installation, WindowPlugin::default())
        }

        fn without_primary(kernel_installation: KernelInstallation) -> Result<Self, String> {
            Self::with_window_plugin(
                kernel_installation,
                WindowPlugin {
                    primary_window: None,
                    ..default()
                },
            )
        }

        fn with_window_plugin(
            kernel_installation: KernelInstallation,
            window_plugin: WindowPlugin,
        ) -> Result<Self, String> {
            let directory = tempdir()
                .map_err(|error| format!("failed to create plugin state directory: {error}"))?;
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, window_plugin))
                .insert_resource(WinitMonitors::default());
            if matches!(kernel_installation, KernelInstallation::Preinstalled) {
                app.add_plugins(RiggingPlugin);
            }
            app.add_plugins(WindowManagerPlugin::with_path(
                directory.path().join("windows.ron"),
            ));
            Ok(Self { app, directory })
        }

        fn install_headless_window_topology(&mut self) {
            self.app.world_mut().spawn(Monitor {
                name:                    None,
                physical_height:         1_080,
                physical_width:          1_920,
                physical_position:       IVec2::ZERO,
                refresh_rate_millihertz: None,
                scale_factor:            1.0,
                video_modes:             Vec::new(),
            });
        }

        fn install_identified_headless_window_topology(&mut self) -> Entity {
            let entity = self
                .app
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
            self.app
                .world_mut()
                .insert_resource(monitors::InjectedMonitorEvidence::identified(
                    entity,
                    IDENTIFIED_PANEL_EVIDENCE,
                ));
            self.app
                .world_mut()
                .insert_resource(monitors::InjectedWinitMonitorOrder::single(entity));
            self.app
                .world_mut()
                .resource_mut::<InjectedFreshWinitDisplays>()
                .entities = vec![entity];
            entity
        }

        /// Install two identified displays side by side, the left one at the desktop origin.
        ///
        /// `install_identified_headless_window_topology` installs one display, which cannot show
        /// whether a binding follows its window: every position resolves against the same monitor.
        fn install_identified_headless_display_pair(&mut self) -> (Entity, Entity) {
            let left = self
                .app
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
            let right = self
                .app
                .world_mut()
                .spawn(Monitor {
                    name:                    None,
                    physical_height:         1_080,
                    physical_width:          1_920,
                    physical_position:       IVec2::new(1_920, 0),
                    refresh_rate_millihertz: None,
                    scale_factor:            1.0,
                    video_modes:             Vec::new(),
                })
                .id();
            self.app.world_mut().insert_resource(
                monitors::InjectedMonitorEvidence::identified_pair(
                    (left, IDENTIFIED_PANEL_EVIDENCE),
                    (right, SECOND_IDENTIFIED_PANEL_EVIDENCE),
                ),
            );
            self.app
                .world_mut()
                .insert_resource(monitors::InjectedWinitMonitorOrder::pair(left, right));
            self.app
                .world_mut()
                .resource_mut::<InjectedFreshWinitDisplays>()
                .entities = vec![left, right];
            (left, right)
        }

        fn write_current_primary_placement(
            &self,
            panel_fingerprint: PanelFingerprint,
        ) -> Result<(), String> {
            let contents = format!(
                "(\n    version: {},\n    entries: [\n        (\n            key: Primary,\n            state: (\n                position: MonitorOffset((40, 30)),\n                logical_width: 640,\n                logical_height: 480,\n                monitor_panel: Fingerprinted(({})),\n                mode: Windowed,\n                app_name: \"startup-test\",\n            ),\n        ),\n    ],\n)\n",
                4,
                panel_fingerprint.get(),
            );
            let PersistedWindowStateDecodeOutcome::Decoded(decoded) =
                persistence::decode_persisted_state_for_test(&contents)
            else {
                return Err(String::from(
                    "current persisted state fixture did not decode",
                ));
            };
            let role = persistence::primary_window_role()
                .map_err(|error| format!("failed to create fixture role: {error}"))?;
            let persisted = decoded
                .get(&role)
                .ok_or_else(|| String::from("decoded fixture has no primary placement"))?;
            if persisted.target
                != PersistedWindowTargetV5::AwaitingLegacyEvidence(
                    PersistedPanelIdentityV4::Fingerprinted(PersistedPanelFingerprintV4(
                        panel_fingerprint.get(),
                    )),
                )
            {
                return Err("decoded fixture did not retain its legacy panel evidence".into());
            }
            fs::write(self.directory.path().join("windows.ron"), contents)
                .map_err(|error| format!("failed to write current persisted state: {error}"))
        }

        /// Write a v2 file, the format that upgrades to a target no live monitor can ever match.
        ///
        /// `convert_v2_state_to_v4` has no panel evidence to carry forward, so every v2 record
        /// becomes `PersistedPanelIdentityV4::Anonymous`. This fixture is that state, asserted
        /// after decode so the test cannot silently start exercising a resolvable target.
        fn write_legacy_anonymous_primary_placement(&self) -> Result<(), String> {
            let contents = format!(
                "(\n    version: {},\n    entries: [\n        (\n            key: Primary,\n            state: (\n                logical_position: Some((40, 30)),\n                logical_width: 640,\n                logical_height: 480,\n                monitor_scale: 1.0,\n                monitor_index: 0,\n                mode: Windowed,\n                app_name: \"startup-test\",\n            ),\n        ),\n    ],\n)\n",
                2,
            );
            let PersistedWindowStateDecodeOutcome::Decoded(decoded) =
                persistence::decode_persisted_state_for_test(&contents)
            else {
                return Err(String::from(
                    "legacy persisted state fixture did not decode",
                ));
            };
            let role = persistence::primary_window_role()
                .map_err(|error| format!("failed to create fixture role: {error}"))?;
            let persisted = decoded
                .get(&role)
                .ok_or_else(|| String::from("decoded fixture has no primary placement"))?;
            if persisted.target
                != PersistedWindowTargetV5::AwaitingLegacyEvidence(
                    PersistedPanelIdentityV4::Anonymous,
                )
            {
                return Err("decoded v2 fixture did not become an anonymous target".into());
            }
            fs::write(self.directory.path().join("windows.ron"), contents)
                .map_err(|error| format!("failed to write legacy persisted state: {error}"))
        }
    }

    fn retirement_binding(app: &mut App, role: RoleKey) -> Result<Binding, String> {
        let authored_id = AuthoredId::new("managed-window-retirement")
            .map_err(|error| format!("failed to create retirement device ID: {error}"))?;
        let driver = app.add_endpoint_driver(KernelRetirementDriver);
        Ok(Binding {
            role,
            endpoint: DeviceEndpoint {
                device: DeviceKey {
                    kind: DeviceKind::Display,
                    id:   DeviceIdSource::Authored { value: authored_id },
                },
                id:     EndpointId::Whole,
            },
            driver,
            recovery: RecoveryPolicy::Forget,
            retry: RetryOn::NewRevision,
            on_abort: OnAbort::default(),
            on_loss: OnSessionLoss::default(),
            state: RoleState::Waiting,
            requested: RequestedConfiguration::new(KernelRetirementConfiguration),
            last_known_good: LastKnownGoodConfiguration::NotEstablished,
            apply_deadline: ApplyDeadline::ProcessDefault,
        })
    }

    fn start_current_exact_identity_apply(
        harness: &mut ProductionPluginHarness,
    ) -> Result<StartedProductionWindowApply, String> {
        let panel_fingerprint =
            monitors::PanelFingerprint::from_evidence_bytes(IDENTIFIED_PANEL_EVIDENCE);
        harness.write_current_primary_placement(panel_fingerprint)?;
        harness.install_identified_headless_window_topology();
        let primary_window = harness
            .app
            .world_mut()
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .iter(harness.app.world())
            .next()
            .ok_or_else(|| String::from("WindowPlugin did not create a primary window"))?;

        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            if harness
                .app
                .world()
                .get::<RestorePreparation>(primary_window)
                .is_some()
            {
                break;
            }
        }

        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let expected_device = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Synthesized {
                digest: Digest::new(panel_fingerprint.get()),
            },
        };
        let binding = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&role)
            .map_err(|error| {
                format!("exact persisted identity did not author a binding: {error}")
            })?;
        assert_eq!(binding.endpoint.device, expected_device);
        let RoleState::Applying(attempt_id) = binding.state else {
            return Err("exact persisted identity did not start the window driver".into());
        };
        let attempts = harness.app.world().resource::<Attempts>();
        let AttemptLookup::InFlight(attempt) = attempts.in_flight(attempt_id) else {
            return Err("window driver attempt was not retained".into());
        };
        assert_eq!(attempt.role, role);
        assert_eq!(attempt.endpoint, binding.endpoint);
        assert!(
            harness
                .app
                .world()
                .get::<RestorePreparation>(primary_window)
                .is_some()
        );
        let configuration = harness
            .app
            .world()
            .get::<WindowApplyConfiguration>(primary_window)
            .ok_or_else(|| String::from("window driver did not install its apply configuration"))?;
        assert_eq!(configuration.placement().logical_size, UVec2::new(640, 480));
        assert_eq!(
            configuration.placement().position,
            persistence::EstablishedWindowPosition::Restorable {
                logical_offset: IVec2::new(40, 30),
            }
        );

        Ok(StartedProductionWindowApply {
            primary_window,
            role,
            attempt_id,
        })
    }

    fn assert_late_primary_reaches_target_preparation(
        component_arrival: ComponentArrival,
    ) -> Result<(), String> {
        let mut harness =
            ProductionPluginHarness::without_primary(KernelInstallation::WindowManagerOwned)?;
        let monitor_entity = harness.install_identified_headless_window_topology();
        harness.app.update();
        assert_no_primary_window(&mut harness.app);

        let primary_window = harness.app.world_mut().spawn(Window::default()).id();
        match component_arrival {
            ComponentArrival::AssociationBeforeRegistration => {
                harness
                    .app
                    .world_mut()
                    .entity_mut(primary_window)
                    .insert(OnMonitor(monitor_entity));
                harness.app.world_mut().flush();
                harness
                    .app
                    .world_mut()
                    .entity_mut(primary_window)
                    .insert(PrimaryWindow);
            },
            ComponentArrival::RegistrationBeforeAssociation => {
                harness
                    .app
                    .world_mut()
                    .entity_mut(primary_window)
                    .insert(PrimaryWindow);
                harness.app.world_mut().flush();
                harness
                    .app
                    .world_mut()
                    .entity_mut(primary_window)
                    .insert(OnMonitor(monitor_entity));
            },
        }

        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            if harness
                .app
                .world()
                .get::<RestorePreparation>(primary_window)
                .is_some()
            {
                break;
            }
        }

        let current_monitor = harness
            .app
            .world()
            .get::<CurrentMonitor>(primary_window)
            .ok_or_else(|| String::from("late primary window has no current monitor"))?;
        let on_monitor = harness
            .app
            .world()
            .get::<OnMonitor>(primary_window)
            .ok_or_else(|| String::from("late primary window lost its monitor association"))?;
        assert!(
            monitors::exact_monitor_association(
                on_monitor,
                current_monitor,
                harness.app.world().resource::<Monitors>(),
            )
            .is_some()
        );
        assert!(
            harness
                .app
                .world()
                .get::<RestorePreparation>(primary_window)
                .is_some()
        );
        assert!(
            harness
                .app
                .world()
                .get::<TargetPosition>(primary_window)
                .is_none()
        );

        harness
            .app
            .world_mut()
            .init_resource::<InjectedWinitWindows>();
        harness
            .app
            .world_mut()
            .resource_mut::<InjectedWinitWindows>()
            .insert(primary_window, UVec2::ZERO);
        harness.app.update();

        assert!(
            harness
                .app
                .world()
                .get::<TargetPosition>(primary_window)
                .is_some()
        );
        Ok(())
    }

    fn assert_no_primary_window(app: &mut App) {
        let mut primary_windows = app
            .world_mut()
            .query_filtered::<Entity, With<PrimaryWindow>>();
        assert!(primary_windows.iter(app.world()).next().is_none());
    }

    fn applying_attempt_id(app: &App, role: &RoleKey) -> Option<AttemptId> {
        let binding = app.world().resource::<Bindings>().binding(role).ok()?;
        match binding.state {
            RoleState::Applying(attempt_id) => Some(attempt_id),
            _ => None,
        }
    }

    fn complete_successful_window_apply_poll(
        harness: &mut ProductionPluginHarness,
        started: &StartedProductionWindowApply,
    ) -> Result<(), String> {
        harness
            .app
            .world_mut()
            .resource_mut::<WindowDriverAttemptResults>()
            .record(started.attempt_id, AttemptOutcome::Succeeded);
        harness.app.update();

        let attempts = harness.app.world().resource::<Attempts>();
        assert!(matches!(
            attempts.in_flight(started.attempt_id),
            AttemptLookup::Finished
        ));
        let binding = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&started.role)
            .map_err(|error| format!("settled window binding disappeared: {error}"))?;
        assert_eq!(binding.state, RoleState::Ready);
        assert!(matches!(
            binding.last_known_good,
            LastKnownGoodConfiguration::NotEstablished
        ));
        Ok(())
    }

    fn assert_finished_attempt_clears_preparation_without_native_window() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let started = start_current_exact_identity_apply(&mut harness)?;

        harness.app.update();

        assert!(
            harness
                .app
                .world()
                .get::<TargetPosition>(started.primary_window)
                .is_none()
        );
        complete_successful_window_apply_poll(&mut harness, &started)?;

        harness.app.update();

        assert!(
            harness
                .app
                .world()
                .get::<RestorePreparation>(started.primary_window)
                .is_none()
        );
        assert!(
            harness
                .app
                .world()
                .get::<WindowApplyConfiguration>(started.primary_window)
                .is_none()
        );
        Ok(())
    }

    fn assert_next_frame_typed_window_readback(
        harness: &mut ProductionPluginHarness,
        started: &StartedProductionWindowApply,
    ) -> Result<(), String> {
        let expected_readback = {
            let world = harness.app.world();
            let window = world
                .get::<Window>(started.primary_window)
                .ok_or_else(|| String::from("primary entity lost its Window before readback"))?;
            let current_monitor = world
                .get::<CurrentMonitor>(started.primary_window)
                .ok_or_else(|| {
                    String::from("primary entity lost its current monitor before readback")
                })?;
            let physical_position = match window.position {
                WindowPosition::At(position) => Some(IVec2::new(position.x, position.y)),
                _ => None,
            };
            persistence::EstablishedWindowPlacement::from_readback(
                window,
                current_monitor,
                physical_position,
                *world.resource::<Platform>(),
            )
        };

        harness.app.update();

        let captured = {
            let bindings = harness.app.world().resource::<Bindings>();
            let binding = bindings
                .binding(&started.role)
                .map_err(|error| format!("readback window binding disappeared: {error}"))?;
            let LastKnownGoodConfiguration::Known(configuration) = &binding.last_known_good else {
                return Err(String::from(
                    "safe readback did not establish a last-known-good configuration",
                ));
            };
            configuration
                .as_any()
                .downcast_ref::<EstablishedWindowPlacement>()
                .cloned()
                .ok_or_else(|| {
                    String::from("safe readback retained the wrong configuration type")
                })?
        };
        assert_eq!(captured, expected_readback);
        Ok(())
    }

    #[test]
    fn window_manager_installs_its_kernel_and_persistence_resources() -> Result<(), String> {
        let harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;

        assert!(harness.app.is_plugin_added::<RiggingPlugin>());
        assert!(harness.app.world().contains_resource::<Bindings>());
        assert!(harness.app.world().contains_resource::<HardwareInventory>());
        assert!(harness.directory.path().is_dir());
        Ok(())
    }

    #[test]
    fn window_manager_reuses_a_preinstalled_kernel() -> Result<(), String> {
        let harness = ProductionPluginHarness::new(KernelInstallation::Preinstalled)?;

        assert!(harness.app.is_plugin_added::<RiggingPlugin>());
        assert!(harness.app.world().contains_resource::<Bindings>());
        assert!(harness.app.world().contains_resource::<HardwareInventory>());
        assert!(harness.directory.path().is_dir());
        Ok(())
    }

    #[test]
    fn managed_window_removal_reaches_the_kernel_retirement_observer() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        harness.install_headless_window_topology();
        harness.app.update();
        let role = persistence::managed_window_role("inspector")
            .map_err(|error| format!("failed to create managed role: {error}"))?;
        let binding = retirement_binding(&mut harness.app, role.clone())?;
        harness
            .app
            .world_mut()
            .resource_mut::<Bindings>()
            .register(binding)
            .map_err(|error| format!("failed to register managed role: {error}"))?;
        let entity = harness
            .app
            .world_mut()
            .spawn(ManagedWindow {
                name: "inspector".into(),
            })
            .id();

        if !harness.app.world_mut().despawn(entity) {
            return Err(String::from("managed window did not despawn"));
        }
        harness.app.world_mut().flush();

        assert!(
            harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&role)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn one_plugin_app_updates_with_live_kernel_resources() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        harness.install_headless_window_topology();

        harness.app.update();

        assert!(harness.app.world().contains_resource::<Bindings>());
        assert!(harness.app.world().contains_resource::<HardwareInventory>());
        Ok(())
    }

    #[test]
    fn current_exact_identity_applies_and_safely_reads_back_with_production_plugins()
    -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        complete_successful_window_apply_poll(&mut harness, &started)?;
        assert_next_frame_typed_window_readback(&mut harness, &started)
    }

    #[test]
    fn late_primary_prepares_when_monitor_association_precedes_registration() -> Result<(), String>
    {
        assert_late_primary_reaches_target_preparation(
            ComponentArrival::AssociationBeforeRegistration,
        )
    }

    #[test]
    fn late_primary_prepares_when_registration_precedes_monitor_association() -> Result<(), String>
    {
        assert_late_primary_reaches_target_preparation(
            ComponentArrival::RegistrationBeforeAssociation,
        )
    }

    #[test]
    fn finished_window_attempt_clears_preparation_without_native_window() -> Result<(), String> {
        assert_finished_attempt_clears_preparation_without_native_window()
    }

    /// A v1 or v2 file must still restore, which requires a `Binding` to exist for its role.
    ///
    /// `device_for_legacy_panel` compares a saved fingerprint against a live one, so the
    /// `Anonymous` target every v1/v2 file upgrades into matches no monitor and
    /// `resolve_pending` can never reclassify it. Without adopting the occupied monitor,
    /// `author_window_binding` returns `None` forever: the window is never restored, and
    /// `project_established` reports `AbsentBinding` so the file is never rewritten either.
    #[test]
    fn anonymous_legacy_placement_adopts_the_monitor_the_window_occupies() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        harness.write_legacy_anonymous_primary_placement()?;
        harness.install_identified_headless_window_topology();
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
        }

        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let panel_fingerprint =
            monitors::PanelFingerprint::from_evidence_bytes(IDENTIFIED_PANEL_EVIDENCE);
        let expected_device = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Synthesized {
                digest: Digest::new(panel_fingerprint.get()),
            },
        };
        let binding = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&role)
            .map_err(|error| {
                format!("anonymous legacy placement did not author a binding: {error}")
            })?;
        assert_eq!(binding.endpoint.device, expected_device);
        Ok(())
    }

    /// Authoring the binding is only half the migration; the record must also be rewritten.
    ///
    /// The legacy target stays `AwaitingLegacyEvidence` in the file until a save rewrites it,
    /// and `project_established` refuses to project a role with no binding. The two halves
    /// therefore fail together, so the restore assertion above cannot stand in for this one.
    #[test]
    fn a_restored_anonymous_legacy_placement_is_rewritten_as_a_classified_record()
    -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        harness.write_legacy_anonymous_primary_placement()?;
        harness.install_identified_headless_window_topology();
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;

        let mut attempt_id = None;
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            if let Some(started) = applying_attempt_id(&harness.app, &role) {
                attempt_id = Some(started);
                break;
            }
        }
        let attempt_id = attempt_id
            .ok_or_else(|| String::from("anonymous legacy placement never started a restore"))?;

        harness
            .app
            .world_mut()
            .resource_mut::<WindowDriverAttemptResults>()
            .record(attempt_id, AttemptOutcome::Succeeded);
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
        }

        let contents = fs::read_to_string(harness.directory.path().join("windows.ron"))
            .map_err(|error| format!("restored state file is unreadable: {error}"))?;
        let PersistedWindowStateDecodeOutcome::Decoded(decoded) =
            persistence::decode_persisted_state_for_test(&contents)
        else {
            return Err(String::from("rewritten state file did not decode"));
        };
        let persisted = decoded
            .get(&role)
            .ok_or_else(|| String::from("rewritten state file has no primary placement"))?;
        assert!(matches!(
            persisted.target,
            persistence::PersistedWindowTargetV5::Classified(_)
        ));
        Ok(())
    }

    /// A window dragged to another display must save against the display it now occupies.
    ///
    /// `write_established_window_configurations` pairs `Binding::endpoint`'s device with an offset
    /// `EstablishedWindowPlacement::from_readback` measured against `CurrentMonitor`. Without
    /// `rebind_window_to_its_current_display` the endpoint keeps the display
    /// `author_window_bindings` authored at startup, so the record names the launch display and
    /// describes a position on the one the window was moved to, and the next restore sends the
    /// window back to the launch display at the right size.
    #[test]
    fn a_window_moved_to_another_display_rebinds_to_it() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let (_, right) = harness.install_identified_headless_display_pair();
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;

        let mut attempt_id = None;
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            if let Some(started) = applying_attempt_id(&harness.app, &role) {
                attempt_id = Some(started);
                break;
            }
        }
        // The move below happens while this apply is still in flight, which is the only ordering a
        // real drag produces: moving a window is itself what puts the role into `Applying`, so a
        // rebind that waits for `RoleState::Ready` never runs. Settling the apply first hid exactly
        // that defect.
        attempt_id.ok_or_else(|| String::from("primary window never started an apply"))?;

        let launch_device = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&role)
            .map_err(|error| format!("primary window is not bound after startup: {error}"))?
            .endpoint
            .device
            .clone();

        let primary_window = {
            let mut primary = harness
                .app
                .world_mut()
                .query_filtered::<Entity, With<PrimaryWindow>>();
            primary
                .iter(harness.app.world())
                .next()
                .ok_or_else(|| String::from("harness installed no primary window"))?
        };
        {
            let mut window = harness
                .app
                .world_mut()
                .get_mut::<Window>(primary_window)
                .ok_or_else(|| String::from("primary window entity lost its Window"))?;
            window.position = WindowPosition::At(IVec2::new(2_020, 100));
        }
        harness
            .app
            .world_mut()
            .entity_mut(primary_window)
            .insert(OnMonitor(right));
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
        }

        let right_fingerprint =
            monitors::PanelFingerprint::from_evidence_bytes(SECOND_IDENTIFIED_PANEL_EVIDENCE);
        let expected_device = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Synthesized {
                digest: Digest::new(right_fingerprint.get()),
            },
        };
        let rebound_device = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&role)
            .map_err(|error| format!("primary window lost its binding: {error}"))?
            .endpoint
            .device
            .clone();
        assert_ne!(rebound_device, launch_device);
        assert_eq!(rebound_device, expected_device);
        Ok(())
    }

    /// A window moved within its display must save the position it was moved to.
    ///
    /// The kernel captures a role's configuration once, when the role becomes ready, and the
    /// persistence projection writes only that captured value. A move that stays on one display
    /// replaces no binding, so without `forget_stale_window_captures` the saved record keeps the
    /// launch-time offset and every relaunch restores the position the window was moved away from.
    /// Two moves make the second one prove the recapture path out of an already-established value,
    /// not just the first capture after startup.
    #[test]
    fn a_window_moved_within_its_display_saves_the_new_offset() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        harness.install_identified_headless_window_topology();
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;

        let mut attempt_id = None;
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            if let Some(started) = applying_attempt_id(&harness.app, &role) {
                attempt_id = Some(started);
                break;
            }
        }
        let attempt_id =
            attempt_id.ok_or_else(|| String::from("primary window never started an apply"))?;
        harness
            .app
            .world_mut()
            .resource_mut::<WindowDriverAttemptResults>()
            .record(attempt_id, AttemptOutcome::Succeeded);
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
        }

        let primary_window = {
            let mut primary = harness
                .app
                .world_mut()
                .query_filtered::<Entity, With<PrimaryWindow>>();
            primary
                .iter(harness.app.world())
                .next()
                .ok_or_else(|| String::from("harness installed no primary window"))?
        };
        for moved_to in [IVec2::new(431, 227), IVec2::new(112, 640)] {
            {
                let mut window = harness
                    .app
                    .world_mut()
                    .get_mut::<Window>(primary_window)
                    .ok_or_else(|| String::from("primary window entity lost its Window"))?;
                window.position = WindowPosition::At(moved_to);
            }
            for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
                harness.app.update();
            }

            let contents = fs::read_to_string(harness.directory.path().join("windows.ron"))
                .map_err(|error| format!("saved state file is unreadable: {error}"))?;
            let PersistedWindowStateDecodeOutcome::Decoded(decoded) =
                persistence::decode_persisted_state_for_test(&contents)
            else {
                return Err(String::from("saved state file did not decode"));
            };
            let persisted = decoded
                .get(&role)
                .ok_or_else(|| String::from("saved state file has no primary placement"))?;
            // The headless display sits at the desktop origin with scale 1, so the persisted
            // monitor offset equals the physical position the window was moved to.
            assert_eq!(
                persisted.position,
                persistence::PersistedPosition::MonitorOffset(moved_to)
            );
        }
        Ok(())
    }

    #[test]
    fn simultaneous_window_attempt_results_are_isolated_by_attempt_id() -> Result<(), String> {
        let mut harness =
            ProductionPluginHarness::without_primary(KernelInstallation::WindowManagerOwned)?;
        let monitor_entity = harness.install_identified_headless_window_topology();
        harness.app.update();

        harness.app.world_mut().spawn((
            Window::default(),
            PrimaryWindow,
            OnMonitor(monitor_entity),
        ));
        harness.app.world_mut().spawn((
            Window::default(),
            ManagedWindow {
                name: "attempt-isolation".into(),
            },
            OnMonitor(monitor_entity),
        ));
        let primary_role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let managed_role = persistence::managed_window_role("attempt-isolation")
            .map_err(|error| format!("failed to create managed role: {error}"))?;

        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            if applying_attempt_id(&harness.app, &primary_role).is_some()
                && applying_attempt_id(&harness.app, &managed_role).is_some()
            {
                break;
            }
        }
        let primary_attempt = applying_attempt_id(&harness.app, &primary_role)
            .ok_or_else(|| String::from("primary window did not start its attempt"))?;
        let managed_attempt = applying_attempt_id(&harness.app, &managed_role)
            .ok_or_else(|| String::from("managed window did not start its attempt"))?;
        assert_ne!(primary_attempt, managed_attempt);

        harness
            .app
            .world_mut()
            .resource_mut::<WindowDriverAttemptResults>()
            .record(primary_attempt, AttemptOutcome::Succeeded);
        harness.app.update();
        assert_eq!(
            harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&primary_role)
                .map_err(|error| format!("primary binding disappeared: {error}"))?
                .state,
            RoleState::Ready,
        );
        assert_eq!(
            applying_attempt_id(&harness.app, &managed_role),
            Some(managed_attempt),
        );
        Ok(())
    }
}
