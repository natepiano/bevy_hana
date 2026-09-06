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
//! 3. **Timing of scale factor updates**: `CachedWindow` is updated after winit events are
//!    processed, so this crate's systems run before the `ScaleFactorChanged` event arrives.
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
//! (it defaults to the executable name).
//!
//! See the `custom_path` example for how to override the full path to the state file.

mod constants;
mod deadline;
#[cfg(any(test, feature = "test"))]
mod display_test_adapter;
mod driver;
#[cfg(test)]
mod driver_conformance_tests;
mod events;
#[cfg(target_os = "macos")]
mod macos_tabbing_fix;
mod managed;
mod monitors;
mod output_proof;
#[cfg(test)]
mod output_proof_tests;
mod persistence;
#[cfg(test)]
mod placement_retry_tests;
mod platform;
mod recovery;
mod reporter;
mod resize_announcement;
mod restore;
mod restore_window_config;
#[cfg(test)]
mod scripted_topology_tests;
mod visibility;
#[cfg(all(target_os = "windows", feature = "workaround-winit-4341"))]
mod windows_dpi_fix;

use std::path::PathBuf;

use bevy::camera::CameraUpdateSystems;
use bevy::prelude::Add;
use bevy::prelude::Added;
use bevy::prelude::App;
#[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
use bevy::prelude::ApplyDeferred;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::On;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::PreStartup;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
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
pub use display_test_adapter::DisplayTestAdapterInstallation;
#[cfg(any(test, feature = "test"))]
pub use display_test_adapter::DisplayTestDescriptor;
#[cfg(any(test, feature = "test"))]
pub use display_test_adapter::DisplayTestDeviceKey;
#[cfg(any(test, feature = "test"))]
pub use display_test_adapter::DisplayTestEnumeration;
#[cfg(any(test, feature = "test"))]
pub use display_test_adapter::DisplayTestReporterLookup;
use driver::WindowDriverId;
use driver::WindowEndpointDriver;
use driver::WindowRoleDriverState;
pub use events::ExpectedLogicalPosition;
pub use events::ExpectedPhysicalPosition;
pub use events::ObservedLogicalPosition;
pub use events::ObservedPhysicalPosition;
pub use events::WindowRestoreMismatch;
pub use events::WindowRestored;
use hana_rigging::prelude::RiggingAppExt;
use hana_rigging::prelude::RiggingPlugin;
use hana_rigging::prelude::RiggingSystems;
pub use managed::ManagedWindow;
pub use managed::ManagedWindowName;
pub use managed::ManagedWindowPersistence;
use managed::ManagedWindowRegistry;
pub use managed::RecoverOnRequest;
pub use managed::RecoverOnReturn;
use managed::WindowRiggingRole;
use managed::author_window_bindings;
use managed::forget_stale_window_captures;
use managed::on_managed_window_added;
use managed::on_managed_window_load;
use managed::on_managed_window_removed;
use managed::on_primary_window_removed;
use managed::on_window_rigging_role_removed;
use managed::queue_window_display_rebind;
use managed::rebind_window_to_its_current_display;
use managed::warn_conflicting_recovery_markers;
pub use monitors::CurrentMonitor;
pub use monitors::CurrentMonitorIndex;
pub use monitors::DisplayEnumerationSource;
pub use monitors::DisplayFingerprint;
pub use monitors::DisplayIdentity;
pub use monitors::DisplayProductName;
pub use monitors::LiveDisplayContradictionCount;
pub use monitors::LiveDisplayDevices;
pub use monitors::LiveDisplayEndpoint;
pub use monitors::LiveDisplayEndpointLookup;
pub use monitors::LiveDisplayMatchError;
pub use monitors::LiveDisplayMonitor;
pub use monitors::LiveMonitor;
#[cfg(not(test))]
#[doc(hidden)]
pub use monitors::LiveWinitProductionBackendSelection;
pub use monitors::MonitorDescriptor;
use monitors::MonitorPlugin;
pub use monitors::MonitorTopologyRevision;
pub use monitors::Monitors;
use persistence::PersistencePlugin;
pub use persistence::managed_window_role;
pub use persistence::primary_window_role;
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

/// Why a managed window's current hidden lifetime has ended.
///
/// The component is removed whenever the window is hidden again, so adding a new disposition
/// announces the end of that specific hidden lifetime.
#[derive(Clone, Copy, Component, Debug, Eq, PartialEq, Reflect)]
#[reflect(Component)]
pub(crate) enum WindowRevealDisposition {
    /// The saved position, size, and window mode were written to the window.
    SavedGeometryApplied,
    /// The role had no saved configuration, so the window kept its opening position.
    NothingSaved,
    /// Saved geometry was fitted inside the display on which the window opened.
    FittedToFallbackDisplay,
    /// Kernel state established that no placement operation will arrive.
    PlacementAbandoned,
}

/// The main plugin. See module docs for usage.
///
/// Default state file locations:
/// - macOS: `~/Library/Application Support/<executable_name>/windows.ron`
/// - Linux: `~/.config/<executable_name>/windows.ron`
/// - Windows: `C:\Users\<User>\AppData\Roaming\<executable_name>\windows.ron`
///
/// Added as `.add_plugins(WindowManagerPlugin)`; it builds a
/// [`ConfiguredWindowManagerPlugin`] with the default state path, the default
/// [`ManagedWindowPersistence`], and no primary-window recovery marker.
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
    pub fn with_app_name(app_name: impl Into<String>) -> ConfiguredWindowManagerPlugin {
        ConfiguredWindowManagerPlugin {
            path:                             persistence::get_state_path_for_app(&app_name.into())
                .expect("Could not determine state file path"),
            managed_window_persistence:       ManagedWindowPersistence::default(),
            primary_window_recovery_behavior: PrimaryWindowRecoveryBehavior::RestoreOnly,
        }
    }

    /// Create a plugin with a custom state file path.
    #[must_use]
    pub fn with_path(path: impl Into<PathBuf>) -> ConfiguredWindowManagerPlugin {
        ConfiguredWindowManagerPlugin {
            path:                             path.into(),
            managed_window_persistence:       ManagedWindowPersistence::default(),
            primary_window_recovery_behavior: PrimaryWindowRecoveryBehavior::RestoreOnly,
        }
    }

    /// Create a plugin with a specific persistence behavior.
    ///
    /// # Panics
    ///
    /// Panics if the config directory cannot be determined.
    #[must_use]
    #[expect(clippy::expect_used, reason = "fail fast if path cannot be determined")]
    pub fn with_persistence(
        managed_window_persistence: ManagedWindowPersistence,
    ) -> ConfiguredWindowManagerPlugin {
        ConfiguredWindowManagerPlugin {
            path: persistence::get_default_state_path()
                .expect("Could not determine state file path"),
            managed_window_persistence,
            primary_window_recovery_behavior: PrimaryWindowRecoveryBehavior::RestoreOnly,
        }
    }
}

impl Plugin for WindowManagerPlugin {
    #[expect(clippy::expect_used, reason = "fail fast if path cannot be determined")]
    fn build(&self, app: &mut App) {
        app.add_plugins(ConfiguredWindowManagerPlugin {
            path:                             persistence::get_default_state_path()
                .expect("Could not determine state file path"),
            managed_window_persistence:       ManagedWindowPersistence::default(),
            primary_window_recovery_behavior: PrimaryWindowRecoveryBehavior::RestoreOnly,
        });
    }
}

/// Configures a [`WindowManagerPlugin`] with persistence and primary-window recovery behavior.
pub struct ConfiguredWindowManagerPlugin {
    path:                             PathBuf,
    managed_window_persistence:       ManagedWindowPersistence,
    primary_window_recovery_behavior: PrimaryWindowRecoveryBehavior,
}

#[derive(Clone, Copy)]
enum PrimaryWindowRecoveryBehavior {
    RestoreOnly,
    RecoverOnReturn,
    RecoverOnRequest,
}

impl ConfiguredWindowManagerPlugin {
    /// Recover the primary window automatically when its display returns.
    #[must_use]
    pub const fn recover_on_return(mut self) -> Self {
        self.primary_window_recovery_behavior = PrimaryWindowRecoveryBehavior::RecoverOnReturn;
        self
    }

    /// Recover the primary window when application code requests it.
    #[must_use]
    pub const fn recover_on_request(mut self) -> Self {
        self.primary_window_recovery_behavior = PrimaryWindowRecoveryBehavior::RecoverOnRequest;
        self
    }

    fn install_window_reveal_systems(app: &mut App) {
        // Chained so the frame that reveals a stranded window also takes the first observation of
        // where it landed; the baseline is closed out on the frame after that.
        app.add_systems(
            Update,
            (
                visibility::abandon_placement_after_deadline,
                show_window_once_placement_settles.after(restore::place_window_at_saved_geometry),
                managed::adopt_live_display_for_stranded_window,
            )
                .chain()
                .after(ClerestoryWindowDriverSet::BindingAuthoring)
                .in_set(ClerestoryUpdateSet::RecoveryWindow),
        );
    }

    fn install_primary_window_recovery_behavior(&self, app: &mut App) {
        match self.primary_window_recovery_behavior {
            PrimaryWindowRecoveryBehavior::RestoreOnly => {},
            PrimaryWindowRecoveryBehavior::RecoverOnReturn => {
                app.add_observer(mark_primary_window_for_recovery_on_return);
                mark_existing_primary_windows_for_recovery_on_return(app);
            },
            PrimaryWindowRecoveryBehavior::RecoverOnRequest => {
                app.add_observer(mark_primary_window_for_recovery_on_request);
                mark_existing_primary_windows_for_recovery_on_request(app);
            },
        }
    }
}

/// The platform this plugin build configures for.
///
/// Production always detects. The crate's own tests pin the resource before adding the plugin,
/// because [`Platform::detect`] reads the environment and would otherwise make one test exercise
/// different branches on different hosts.
#[cfg(test)]
fn configured_platform(app: &App) -> Platform {
    app.world()
        .get_resource::<Platform>()
        .copied()
        .unwrap_or_else(Platform::detect)
}

/// The platform this plugin build configures for.
#[cfg(not(test))]
#[allow(
    clippy::missing_const_for_fn,
    reason = "`Platform::detect` is only `const` on macOS and Windows; on Linux it reads the               environment, so a `const fn` here would not compile there"
)]
fn configured_platform(_: &App) -> Platform { Platform::detect() }

#[cfg(target_os = "macos")]
fn install_macos_window_tabbing_fix(app: &mut App) {
    // App-wide opt-out of automatic window tabbing, before winit creates any OS window. See
    // `macos_tabbing_fix` module docs.
    macos_tabbing_fix::disable_automatic_tabbing();
    app.add_systems(
        Update,
        macos_tabbing_fix::disable_tabbing_on_managed
            .before(restore::place_window_at_saved_geometry),
    );
}

impl Plugin for ConfiguredWindowManagerPlugin {
    fn build(&self, app: &mut App) {
        output_proof::register_window_output_proof(app);
        let managed_window_persistence = self.managed_window_persistence.clone();
        if !app.is_plugin_added::<RiggingPlugin>() {
            app.add_plugins(RiggingPlugin);
        }
        app.register_rigging_role_relationship::<WindowRiggingRole>();
        let window_driver = app.add_endpoint_driver(WindowEndpointDriver);
        let platform = configured_platform(app);
        app.insert_resource(platform);
        hide_startup_window(app, platform);
        #[cfg(target_os = "macos")]
        install_macos_window_tabbing_fix(app);

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
        .init_resource::<WindowRoleDriverState>()
        .insert_resource(RestoreWindowConfig {
            path: self.path.clone(),
        })
        .insert_resource(managed_window_persistence)
        .init_resource::<ManagedWindowRegistry>()
        .add_observer(on_managed_window_added)
        .add_observer(on_managed_window_removed)
        .add_observer(on_primary_window_removed)
        .add_observer(on_window_rigging_role_removed)
        .add_observer(on_managed_window_load)
        .add_observer(mark_primary_window_as_managed)
        .add_observer(warn_conflicting_recovery_markers)
        .add_observer(queue_window_display_rebind)
        .add_observer(visibility::resume_placement_when_wanted_display_returns);
        mark_existing_primary_windows_as_managed(app);
        self.install_primary_window_recovery_behavior(app);

        app.add_systems(
            Update,
            (
                author_window_bindings,
                rebind_window_to_its_current_display,
                forget_stale_window_captures.after(RiggingSystems::Reconcile),
            )
                .chain()
                .after(RiggingSystems::Reconcile)
                .in_set(ClerestoryWindowDriverSet::BindingAuthoring),
        );

        Self::install_window_reveal_systems(app);

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
                    .before(restore::place_window_at_saved_geometry)
                    .in_set(ClerestoryWindowDriverSet::TargetPreparation),
                ApplyDeferred
                    .after(restore::compensate_target_position)
                    .in_set(ClerestoryWindowDriverSet::TargetPreparation),
                // Re-apply the compensated position once the window is mapped: bevy 0.19
                // can ignore the first `set_outer_position` request while the X11 window is
                // unmapped, while a mapped window's `Window.position` readback matches the
                // requested compensated position plus `X11FrameTop`.
                restore::reapply_compensated_position
                    .after(restore::place_window_at_saved_geometry)
                    .before(restore::check_restore_settling)
                    .in_set(ClerestoryWindowDriverSet::TargetPreparation),
            )
                .run_if(|p: Res<Platform>| p.is_x11()),
        );
    }
}

/// Carry the primary window into the managed set the rest of the crate queries.
///
/// Applications never add [`ManagedWindow`] to the primary window, so every `With<ManagedWindow>`
/// query would skip it without this. The paired build-time pass covers a primary window that
/// `WindowPlugin` spawned before this observer was registered.
fn mark_primary_window_as_managed(added: On<Add, PrimaryWindow>, mut commands: Commands) {
    commands.entity(added.entity).insert(ManagedWindow);
}

fn mark_existing_primary_windows_as_managed(app: &mut App) {
    let primary_windows = {
        let world = app.world_mut();
        let mut query = world.query_filtered::<Entity, With<PrimaryWindow>>();
        query.iter(world).collect::<Vec<_>>()
    };
    for primary_window in primary_windows {
        app.world_mut()
            .entity_mut(primary_window)
            .insert(ManagedWindow);
    }
}

fn mark_primary_window_for_recovery_on_return(
    added: On<Add, PrimaryWindow>,
    mut commands: Commands,
) {
    commands.entity(added.entity).insert(RecoverOnReturn);
}

fn mark_primary_window_for_recovery_on_request(
    added: On<Add, PrimaryWindow>,
    mut commands: Commands,
) {
    commands.entity(added.entity).insert(RecoverOnRequest);
}

fn mark_existing_primary_windows_for_recovery_on_return(app: &mut App) {
    let primary_windows = {
        let world = app.world_mut();
        let mut query = world.query_filtered::<Entity, With<PrimaryWindow>>();
        query.iter(world).collect::<Vec<_>>()
    };
    for primary_window in primary_windows {
        app.world_mut()
            .entity_mut(primary_window)
            .insert(RecoverOnReturn);
    }
}

fn mark_existing_primary_windows_for_recovery_on_request(app: &mut App) {
    let primary_windows = {
        let world = app.world_mut();
        let mut query = world.query_filtered::<Entity, With<PrimaryWindow>>();
        query.iter(world).collect::<Vec<_>>()
    };
    for primary_window in primary_windows {
        app.world_mut()
            .entity_mut(primary_window)
            .insert(RecoverOnRequest);
    }
}

/// Hide an existing primary immediately; the observer hides a `PrimaryWindow` created later.
/// Under X11 frame compensation nothing is hidden, because the window must stay mapped for
/// `_NET_FRAME_EXTENTS` to be readable.
fn hide_startup_window(app: &mut App, platform: Platform) {
    if platform.should_hide_on_startup() {
        app.add_observer(visibility::hide_window_on_creation);
        let existing_primary = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<(Entity, &mut Window), With<PrimaryWindow>>();
            query.iter_mut(world).next().map(|(entity, mut window)| {
                debug!("[build] Window already exists, hiding immediately");
                window.visible = false;
                entity
            })
        };
        if let Some(entity) = existing_primary {
            let mut entity = app.world_mut().entity_mut(entity);
            entity.remove::<WindowRevealDisposition>();
            entity.insert(visibility::SavedDisplayRevealWait::default());
        }
    } else {
        debug!("[build] Linux X11: skipping window hide for frame extent compensation");
    }
}

fn show_window_once_placement_settles(
    mut windows: Query<(&mut Window, &WindowRevealDisposition), Added<WindowRevealDisposition>>,
) {
    for (mut window, disposition) in &mut windows {
        debug!("[show_window_once_placement_settles] revealing window after {disposition:?}");
        window.visible = true;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;
    use std::time::Duration;

    use bevy::MinimalPlugins;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::Add;
    use bevy::prelude::ApplyDeferred;
    use bevy::prelude::Commands;
    use bevy::prelude::Component;
    use bevy::prelude::Entity;
    use bevy::prelude::IVec2;
    use bevy::prelude::On;
    use bevy::prelude::Plugin;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectComponent;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Startup;
    use bevy::prelude::Time;
    use bevy::prelude::UVec2;
    use bevy::prelude::WindowPosition;
    use bevy::prelude::World;
    use bevy::prelude::default;
    use bevy::reflect::TypePath;
    use bevy::reflect::TypeRegistration;
    use bevy::time::TimeUpdateStrategy;
    use bevy::window::Monitor;
    use bevy::window::OnMonitor;
    use bevy::window::WindowPlugin;
    use bevy::winit::WinitMonitors;
    use hana_rigging::WaitingWork;
    use hana_rigging::prelude::Applied;
    use hana_rigging::prelude::ApplyContext;
    use hana_rigging::prelude::ApplyDeadline;
    use hana_rigging::prelude::AttemptEnding;
    use hana_rigging::prelude::AttemptInvalidation;
    use hana_rigging::prelude::AttemptLookup;
    use hana_rigging::prelude::AttemptRef;
    use hana_rigging::prelude::AuthoredId;
    use hana_rigging::prelude::BindingAuthoring;
    use hana_rigging::prelude::BindingPolicy;
    use hana_rigging::prelude::Bindings;
    use hana_rigging::prelude::CapabilityProjectionFailure;
    use hana_rigging::prelude::DeviceEndpoint;
    use hana_rigging::prelude::DeviceIdSource;
    use hana_rigging::prelude::DeviceKey;
    use hana_rigging::prelude::DeviceKind;
    use hana_rigging::prelude::DeviceStatus;
    use hana_rigging::prelude::Digest;
    use hana_rigging::prelude::DiscoveryControl;
    use hana_rigging::prelude::DriverCleanupRoleEntity;
    use hana_rigging::prelude::DriverCompletion;
    use hana_rigging::prelude::EndpointDriver;
    use hana_rigging::prelude::EndpointId;
    use hana_rigging::prelude::EstablishedContext;
    use hana_rigging::prelude::HardwareInventory;
    use hana_rigging::prelude::KeyAvailability;
    use hana_rigging::prelude::LastKnownGoodConfiguration;
    use hana_rigging::prelude::OnAbort;
    use hana_rigging::prelude::OnSessionLoss;
    use hana_rigging::prelude::PartName;
    use hana_rigging::prelude::ReapplyConfiguration;
    use hana_rigging::prelude::RecoveryPolicy;
    use hana_rigging::prelude::RetryOn;
    use hana_rigging::prelude::RiggingAppExt;
    use hana_rigging::prelude::RiggingLimits;
    use hana_rigging::prelude::RoleKey;
    use hana_rigging::prelude::RoleStatus;
    use hana_rigging::prelude::RoleStatusView;
    use hana_rigging::prelude::SessionLookup;
    use hana_rigging::prelude::SessionRef;
    use hana_rigging::prelude::SessionReleaseCause;
    use hana_rigging::prelude::TargetResolution;
    use hana_rigging::prelude::TargetResolutionContext;
    use hana_rigging::prelude::WaitingStatusView;
    use hana_rigging::prelude::register_binding;
    use hana_rigging::prelude::replace_binding;
    use managed::RecoveryMarkerConflictWarned;
    use managed::WindowBindingAuthoring;
    use managed::WindowRiggingRole;
    use monitors::DisplayFingerprint;
    use monitors::MonitorReporterId;
    use persistence::EstablishedWindowPlacement;
    use persistence::PersistedDisplayFingerprintV4;
    use persistence::PersistedDisplayIdentityV4;
    use persistence::PersistedWindowStateDecodeOutcome;
    use persistence::PersistedWindowTargetV5;
    use recovery::StrandedWindowMovementBaselines;
    use recovery::WindowFallbackRecoveryPhase;
    use recovery::WindowFallbackRecoveryProgress;
    use reporter::InjectedFreshWinitDisplays;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::*;
    use crate::constants::EXACT_DISPLAY_WAIT_TIMEOUT_SECS;
    use crate::constants::SETTLE_STABILITY_SECS;
    use crate::driver::RestoreRecord;
    use crate::restore::InjectedWinitWindows;
    use crate::restore::RestorePreparationSource;
    use crate::restore::TargetPosition;
    use crate::restore::WindowRestoreAttempt;

    const ABSENT_IDENTIFIED_DISPLAY_EVIDENCE: &[u8] = b"absent-startup-display";
    const IDENTIFIED_DISPLAY_EVIDENCE: &[u8] = b"startup-display";
    const SECOND_IDENTIFIED_DISPLAY_EVIDENCE: &[u8] = b"startup-display-two";
    const LEFT_DISPLAY_WINDOW_POSITION: IVec2 = IVec2::new(100, 100);
    const RECOVERY_TOPOLOGY_UPDATE_LIMIT: usize = 32;
    const RIGHT_DISPLAY_WINDOW_POSITION: IVec2 = IVec2::new(2_020, 100);
    const RIGHT_DISPLAY_USER_MOVE_POSITION: IVec2 = IVec2::new(2_300, 200);
    const RIGHTMOST_DISPLAY_WINDOW_POSITION: IVec2 = IVec2::new(3_940, 100);
    const RIGHTMOST_IDENTIFIED_DISPLAY_EVIDENCE: &[u8] = b"startup-display-three";
    const TEST_APPLY_DEADLINE: Duration = Duration::from_millis(10);
    const TEST_APPLY_OVERRUN: Duration = Duration::from_millis(5);
    const STARTUP_RESTORE_UPDATE_LIMIT: usize = 8;

    #[test]
    fn readme_multi_window_example_compiles() {
        fn spawn_windows(mut commands: Commands) {
            commands.spawn((
                Window {
                    title: "Inspector".into(),
                    ..default()
                },
                ManagedWindowName("inspector".into()),
                RecoverOnRequest,
            ));
            commands.spawn((
                Window {
                    title: "Dashboard".into(),
                    ..default()
                },
                ManagedWindowName("dashboard".into()),
                RecoverOnReturn,
            ));
        }

        let mut app = App::new();
        app.add_plugins(WindowManagerPlugin::with_app_name("my-app").recover_on_return())
            .add_systems(Startup, spawn_windows);
    }

    #[test]
    fn the_plugin_manages_the_primary_window_it_finds_and_the_one_it_meets() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let existing_primary = harness.primary_window()?;
        assert!(
            harness
                .app
                .world()
                .get::<ManagedWindow>(existing_primary)
                .is_some()
        );

        let later_primary = harness
            .app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        harness.app.world_mut().flush();

        assert!(
            harness
                .app
                .world()
                .get::<ManagedWindow>(later_primary)
                .is_some()
        );
        Ok(())
    }

    #[test]
    fn configured_window_manager_retargets_a_managed_window_after_its_role_entity_is_despawned()
    -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let primary_window = harness.primary_window()?;
        harness.install_identified_headless_window_topology();
        for _ in 0..3 {
            harness.app.update();
        }
        let despawned_role_entity = harness
            .app
            .world()
            .get::<WindowRiggingRole>(primary_window)
            .ok_or_else(|| String::from("the managed window has no rigging role relationship"))?
            .entity();

        if !harness.app.world_mut().despawn(despawned_role_entity) {
            return Err(String::from(
                "the managed window role entity did not despawn",
            ));
        }
        harness.app.update();

        let replacement_role_entity = harness
            .app
            .world()
            .get::<WindowRiggingRole>(primary_window)
            .ok_or_else(|| {
                String::from("the managed window relationship was not restored after role recovery")
            })?
            .entity();
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        assert_ne!(replacement_role_entity, despawned_role_entity);
        assert_eq!(
            harness
                .app
                .world()
                .resource::<Bindings>()
                .role_entity(&role),
            Ok(replacement_role_entity)
        );
        assert_eq!(
            harness.app.world().get::<RoleKey>(replacement_role_entity),
            Some(&role)
        );
        Ok(())
    }

    #[derive(Clone, Component, Debug, PartialEq, Eq, Reflect)]
    #[reflect(Component, PartialEq)]
    struct KernelRetirementConfiguration;

    struct KernelRetirementDriver;

    impl EndpointDriver for KernelRetirementDriver {
        type Configuration = KernelRetirementConfiguration;
        type Target = ();

        fn resolve_target(
            &mut self,
            _: &mut World,
            _: &TargetResolutionContext<'_>,
            _: &Self::Configuration,
        ) -> TargetResolution<Self::Target> {
            TargetResolution::Reached(())
        }

        fn start_apply(
            &mut self,
            _: &mut World,
            context: ApplyContext<'_, Self::Configuration>,
            _: &Self::Configuration,
            (): Self::Target,
        ) {
            context
                .into_completion()
                .finish(DriverCompletion::Succeeded(Applied::AsDispatched));
        }

        fn established(&mut self, _: &mut World, _: EstablishedContext<'_, Self::Configuration>) {}

        fn cancel_apply(
            &mut self,
            _: &mut World,
            _: &RoleKey,
            _: DriverCleanupRoleEntity,
            _: AttemptRef,
            _: AttemptInvalidation,
        ) {
        }

        fn release_session(
            &mut self,
            _: &mut World,
            _: &RoleKey,
            _: DriverCleanupRoleEntity,
            _: SessionRef,
            _: SessionReleaseCause,
        ) {
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

    #[derive(Clone, Copy, Resource)]
    enum LateManagedWindowRegistration {
        Pending { monitor: Entity },
        Registered { window: Entity },
    }

    impl LateManagedWindowRegistration {
        fn window(self) -> Result<Entity, String> {
            match self {
                Self::Pending { .. } => Err(String::from("managed window was not registered")),
                Self::Registered { window } => Ok(window),
            }
        }
    }

    #[derive(Clone, Copy)]
    enum PrimaryRecoveryMarkerState {
        Absent,
        RecoverOnRequest,
        RecoverOnReturn,
    }

    impl PrimaryRecoveryMarkerState {
        const fn recovery_policy(self) -> RecoveryPolicy {
            match self {
                Self::Absent => RecoveryPolicy::Forget,
                Self::RecoverOnRequest => RecoveryPolicy::ReapplyOnRequest,
                Self::RecoverOnReturn => RecoveryPolicy::ReapplyOnReturn,
            }
        }

        const fn departure_work(self) -> WaitingWork {
            match self {
                Self::Absent => WaitingWork::Nothing,
                Self::RecoverOnRequest => WaitingWork::ReapplyRequestOwed,
                Self::RecoverOnReturn => WaitingWork::RestorationOwed,
            }
        }

        const fn unavailability_work(self) -> WaitingWork {
            match self {
                Self::Absent => WaitingWork::RegistrationOwed,
                Self::RecoverOnRequest | Self::RecoverOnReturn => self.departure_work(),
            }
        }

        fn configure_window_manager(self, path: PathBuf) -> ConfiguredWindowManagerPlugin {
            match self {
                Self::Absent => WindowManagerPlugin::with_path(path),
                Self::RecoverOnRequest => WindowManagerPlugin::with_path(path).recover_on_request(),
                Self::RecoverOnReturn => WindowManagerPlugin::with_path(path).recover_on_return(),
            }
        }
    }

    struct DepartedPrimaryWindow {
        primary_window:   Entity,
        role:             RoleKey,
        departed_device:  DeviceKey,
        survivor_device:  DeviceKey,
        survivor_monitor: Entity,
    }

    struct StartedProductionWindowApply {
        primary_window: Entity,
        role:           RoleKey,
        attempt:        AttemptRef,
    }

    #[derive(Resource)]
    struct ScheduledWindowCompletion {
        attempt: AttemptRef,
        window:  Entity,
    }

    #[derive(Default, Resource)]
    struct PrimaryRecoveryMarkerWarnings(usize);

    fn record_primary_recovery_marker_warning(
        _added: On<Add, RecoveryMarkerConflictWarned>,
        mut primary_recovery_marker_warnings: ResMut<PrimaryRecoveryMarkerWarnings>,
    ) {
        primary_recovery_marker_warnings.0 += 1;
    }

    fn finish_scheduled_window_completion(world: &mut World) {
        let Some(scheduled) = world.remove_resource::<ScheduledWindowCompletion>() else {
            return;
        };
        world
            .resource_mut::<WindowRoleDriverState>()
            .finish_as_dispatched(scheduled.attempt);
        restore::remove_window_restore_work(world, scheduled.window, scheduled.attempt);
    }

    fn remove_live_display_before_window_resolution(world: &mut World) {
        let live_displays = world
            .query_filtered::<Entity, With<LiveDisplayEndpoint>>()
            .iter(world)
            .collect::<Vec<_>>();
        for live_display in live_displays {
            world
                .entity_mut(live_display)
                .remove::<LiveDisplayEndpoint>();
        }
    }

    fn remove_primary_window_role_before_resolution(world: &mut World) {
        let primary_windows = world
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .iter(world)
            .collect::<Vec<_>>();
        for primary_window in primary_windows {
            world
                .entity_mut(primary_window)
                .remove::<WindowRiggingRole>();
        }
    }

    fn prepare_window_target_with_monitors(
        harness: &mut ProductionPluginHarness,
        window: Entity,
        current_monitor_entity: Entity,
        current_monitor_descriptor: MonitorDescriptor,
        monitors: Monitors,
    ) -> Result<(), String> {
        let effective_window_mode = harness
            .app
            .world()
            .get::<CurrentMonitor>(window)
            .ok_or_else(|| String::from("window has no current monitor before target preparation"))?
            .effective_window_mode;
        let world = harness.app.world_mut();
        world.insert_resource(monitors);
        world.entity_mut(window).insert((
            OnMonitor(current_monitor_entity),
            CurrentMonitor {
                descriptor: current_monitor_descriptor,
                effective_window_mode,
            },
        ));
        world.init_resource::<InjectedWinitWindows>();
        world
            .resource_mut::<InjectedWinitWindows>()
            .insert(window, UVec2::ZERO);
        world
            .run_system_once(restore::prepare_driver_restore_targets)
            .map_err(|error| format!("failed to prepare the window target: {error}"))
    }

    fn register_late_managed_window(
        mut commands: Commands,
        mut registration: ResMut<LateManagedWindowRegistration>,
        mut time: ResMut<Time>,
    ) {
        let LateManagedWindowRegistration::Pending { monitor } = *registration else {
            return;
        };
        let mut window = Window {
            position: WindowPosition::At(RIGHT_DISPLAY_WINDOW_POSITION),
            ..default()
        };
        window.resolution.set(640.0, 480.0);
        let window = commands
            .spawn((
                window,
                ManagedWindowName("late-inspector".into()),
                OnMonitor(monitor),
            ))
            .id();
        time.advance_by(Duration::from_secs_f32(EXACT_DISPLAY_WAIT_TIMEOUT_SECS));
        *registration = LateManagedWindowRegistration::Registered { window };
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

        fn with_short_apply_bounds() -> Result<Self, String> {
            Self::with_window_plugin_and_window_manager(
                KernelInstallation::WindowManagerOwned,
                WindowPlugin::default(),
                |app| {
                    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
                    app.insert_resource(RiggingLimits {
                        apply_deadline: TEST_APPLY_DEADLINE,
                        apply_overrun: TEST_APPLY_OVERRUN,
                        ..default()
                    });
                },
                WindowManagerPlugin::with_path,
            )
        }

        fn with_window_plugin(
            kernel_installation: KernelInstallation,
            window_plugin: WindowPlugin,
        ) -> Result<Self, String> {
            Self::with_window_plugin_and_window_manager(
                kernel_installation,
                window_plugin,
                |_| {},
                WindowManagerPlugin::with_path,
            )
        }

        fn with_window_plugin_and_window_manager<P: Plugin>(
            kernel_installation: KernelInstallation,
            window_plugin: WindowPlugin,
            prepare_existing_primary_window: impl FnOnce(&mut App),
            configure_window_manager: impl FnOnce(PathBuf) -> P,
        ) -> Result<Self, String> {
            let directory = tempdir()
                .map_err(|error| format!("failed to create plugin state directory: {error}"))?;
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, window_plugin))
                .insert_resource(WinitMonitors::default())
                .init_resource::<InjectedFreshWinitDisplays>();
            if matches!(kernel_installation, KernelInstallation::Preinstalled) {
                app.add_plugins(RiggingPlugin);
            }
            prepare_existing_primary_window(&mut app);
            // Pin the platform before the plugin builds, so a production-plugin test asserts the
            // same branches on every host. `Platform::detect` reads the environment: on a
            // headless Linux runner it reports `X11`, whose windowed restore then waits forever
            // for the `_NET_FRAME_EXTENTS` reply that gates `X11FrameCompensated`, leaving
            // `RestorePreparation` on the window and `update_current_monitor` skipping it.
            app.insert_resource(Platform::MacOs);
            app.add_plugins(configure_window_manager(
                directory.path().join("windows.ron"),
            ));
            Ok(Self { app, directory })
        }

        fn primary_window(&mut self) -> Result<Entity, String> {
            let mut primary_windows = self
                .app
                .world_mut()
                .query_filtered::<Entity, With<PrimaryWindow>>();
            primary_windows
                .iter(self.app.world())
                .next()
                .ok_or_else(|| String::from("WindowPlugin did not create a primary window"))
        }

        fn primary_recovery_policy(&self) -> Result<RecoveryPolicy, String> {
            let role = persistence::primary_window_role()
                .map_err(|error| format!("failed to create primary role: {error}"))?;
            self.app
                .world()
                .resource::<Bindings>()
                .binding(&role)
                .map(|binding| binding.recovery)
                .map_err(|error| format!("primary window binding was not authored: {error}"))
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
                    IDENTIFIED_DISPLAY_EVIDENCE,
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
                    (left, IDENTIFIED_DISPLAY_EVIDENCE),
                    (right, SECOND_IDENTIFIED_DISPLAY_EVIDENCE),
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

        fn disconnect_identified_display(
            &mut self,
            departed: Entity,
            survivor: Entity,
        ) -> Result<(), String> {
            if !self.app.world_mut().despawn(departed) {
                return Err(String::from("identified display did not despawn"));
            }
            self.app
                .world_mut()
                .insert_resource(monitors::InjectedMonitorEvidence::identified(
                    survivor,
                    SECOND_IDENTIFIED_DISPLAY_EVIDENCE,
                ));
            self.app
                .world_mut()
                .insert_resource(monitors::InjectedWinitMonitorOrder::single(survivor));
            self.app
                .world_mut()
                .resource_mut::<InjectedFreshWinitDisplays>()
                .entities = vec![survivor];
            Ok(())
        }

        fn report_identified_display_departure(&mut self, survivor: Entity) -> Result<(), String> {
            self.app
                .world_mut()
                .resource_mut::<InjectedFreshWinitDisplays>()
                .entities = vec![survivor];
            let reporter = self.app.world().resource::<MonitorReporterId>().get();
            self.app
                .world_mut()
                .resource_mut::<DiscoveryControl>()
                .request(reporter)
                .map_err(|error| format!("failed to request display departure report: {error}"))
        }

        fn reconnect_identified_left_display(&mut self, survivor: Entity) -> Entity {
            let returned = self
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
            self.app.world_mut().insert_resource(
                monitors::InjectedMonitorEvidence::identified_pair(
                    (returned, IDENTIFIED_DISPLAY_EVIDENCE),
                    (survivor, SECOND_IDENTIFIED_DISPLAY_EVIDENCE),
                ),
            );
            self.app
                .world_mut()
                .insert_resource(monitors::InjectedWinitMonitorOrder::pair(
                    returned, survivor,
                ));
            self.app
                .world_mut()
                .resource_mut::<InjectedFreshWinitDisplays>()
                .entities = vec![returned, survivor];
            returned
        }

        fn add_identified_rightmost_display(&mut self, existing: Entity) -> Entity {
            let rightmost = self
                .app
                .world_mut()
                .spawn(Monitor {
                    name:                    None,
                    physical_height:         1_080,
                    physical_width:          1_920,
                    physical_position:       IVec2::new(3_840, 0),
                    refresh_rate_millihertz: None,
                    scale_factor:            1.0,
                    video_modes:             Vec::new(),
                })
                .id();
            self.app.world_mut().insert_resource(
                monitors::InjectedMonitorEvidence::identified_pair(
                    (existing, SECOND_IDENTIFIED_DISPLAY_EVIDENCE),
                    (rightmost, RIGHTMOST_IDENTIFIED_DISPLAY_EVIDENCE),
                ),
            );
            self.app
                .world_mut()
                .insert_resource(monitors::InjectedWinitMonitorOrder::pair(
                    existing, rightmost,
                ));
            self.app
                .world_mut()
                .resource_mut::<InjectedFreshWinitDisplays>()
                .entities = vec![existing, rightmost];
            rightmost
        }

        fn write_current_primary_placement(
            &self,
            display_fingerprint: DisplayFingerprint,
        ) -> Result<(), String> {
            let contents = format!(
                "(\n    version: {},\n    entries: [\n        (\n            key: Primary,\n            state: (\n                position: MonitorOffset((40, 30)),\n                logical_width: 640,\n                logical_height: 480,\n                monitor_panel: Fingerprinted(({})),\n                mode: Windowed,\n                app_name: \"startup-test\",\n            ),\n        ),\n    ],\n)\n",
                4,
                display_fingerprint.get(),
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
                    PersistedDisplayIdentityV4::Fingerprinted(PersistedDisplayFingerprintV4(
                        display_fingerprint.get(),
                    )),
                )
            {
                return Err("decoded fixture did not retain its legacy display evidence".into());
            }
            fs::write(self.directory.path().join("windows.ron"), contents)
                .map_err(|error| format!("failed to write current persisted state: {error}"))
        }

        fn write_absent_classified_primary_placement(&self) -> Result<DeviceKey, String> {
            let display_fingerprint =
                DisplayFingerprint::from_evidence_bytes(ABSENT_IDENTIFIED_DISPLAY_EVIDENCE);
            let contents = format!(
                "(\n    version: 5,\n    entries: [\n        (\n            key: Primary,\n            state: (\n                target: Classified((kind: Display, id: Synthesized(digest: {}))),\n                position: MonitorOffset(({}, {})),\n                logical_width: 640,\n                logical_height: 480,\n                mode: Windowed,\n                app_name: \"startup-test\",\n            ),\n        ),\n    ],\n)\n",
                display_fingerprint.get(),
                LEFT_DISPLAY_WINDOW_POSITION.x,
                LEFT_DISPLAY_WINDOW_POSITION.y,
            );
            let PersistedWindowStateDecodeOutcome::Decoded(decoded) =
                persistence::decode_persisted_state_for_test(&contents)
            else {
                return Err(String::from(
                    "absent classified state fixture did not decode",
                ));
            };
            let role = persistence::primary_window_role()
                .map_err(|error| format!("failed to create fixture role: {error}"))?;
            let persisted = decoded
                .get(&role)
                .ok_or_else(|| String::from("decoded fixture has no primary placement"))?;
            let device = device_for_identified_display(ABSENT_IDENTIFIED_DISPLAY_EVIDENCE);
            if persisted.target != PersistedWindowTargetV5::Classified(device.clone()) {
                return Err(String::from(
                    "decoded fixture did not retain its absent classified display",
                ));
            }
            fs::write(self.directory.path().join("windows.ron"), contents)
                .map_err(|error| format!("failed to write absent classified state: {error}"))?;
            Ok(device)
        }

        fn write_absent_classified_managed_placement(&self) -> Result<DeviceKey, String> {
            let display_fingerprint =
                DisplayFingerprint::from_evidence_bytes(ABSENT_IDENTIFIED_DISPLAY_EVIDENCE);
            let contents = format!(
                "(\n    version: 5,\n    entries: [\n        (\n            key: Managed(\"late-inspector\"),\n            state: (\n                target: Classified((kind: Display, id: Synthesized(digest: {}))),\n                position: MonitorOffset(({}, {})),\n                logical_width: 640,\n                logical_height: 480,\n                mode: Windowed,\n                app_name: \"startup-test\",\n            ),\n        ),\n    ],\n)\n",
                display_fingerprint.get(),
                LEFT_DISPLAY_WINDOW_POSITION.x,
                LEFT_DISPLAY_WINDOW_POSITION.y,
            );
            let PersistedWindowStateDecodeOutcome::Decoded(decoded) =
                persistence::decode_persisted_state_for_test(&contents)
            else {
                return Err(String::from(
                    "absent managed classified state fixture did not decode",
                ));
            };
            let role = persistence::managed_window_role("late-inspector")
                .map_err(|error| format!("failed to create managed fixture role: {error}"))?;
            let persisted = decoded
                .get(&role)
                .ok_or_else(|| String::from("decoded fixture has no managed placement"))?;
            let device = device_for_identified_display(ABSENT_IDENTIFIED_DISPLAY_EVIDENCE);
            if persisted.target != PersistedWindowTargetV5::Classified(device.clone()) {
                return Err(String::from(
                    "decoded fixture did not retain its absent managed display",
                ));
            }
            fs::write(self.directory.path().join("windows.ron"), contents)
                .map_err(|error| format!("failed to write absent managed state: {error}"))?;
            Ok(device)
        }

        /// Write a v2 file, the format that upgrades to a target no live monitor can ever match.
        ///
        /// `convert_v2_state_to_v4` has no display evidence to carry forward, so every v2 record
        /// becomes `PersistedDisplayIdentityV4::Anonymous`. This fixture is that state, asserted
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
                    PersistedDisplayIdentityV4::Anonymous,
                )
            {
                return Err("decoded v2 fixture did not become an anonymous target".into());
            }
            fs::write(self.directory.path().join("windows.ron"), contents)
                .map_err(|error| format!("failed to write legacy persisted state: {error}"))
        }
    }

    fn retirement_binding(
        app: &mut App,
        role: RoleKey,
    ) -> Result<BindingAuthoring<KernelRetirementConfiguration>, String> {
        let authored_id = AuthoredId::new("managed-window-retirement")
            .map_err(|error| format!("failed to create retirement device ID: {error}"))?;
        let driver = app.add_endpoint_driver(KernelRetirementDriver);
        Ok(BindingAuthoring::new(
            role,
            DeviceEndpoint {
                device: DeviceKey {
                    kind: DeviceKind::Display,
                    id:   DeviceIdSource::Authored { value: authored_id },
                },
                id:     EndpointId::Whole,
            },
            driver,
            KernelRetirementConfiguration,
            BindingPolicy::new(
                RecoveryPolicy::Forget,
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        ))
    }

    fn start_current_exact_identity_apply(
        harness: &mut ProductionPluginHarness,
    ) -> Result<StartedProductionWindowApply, String> {
        let display_fingerprint =
            monitors::DisplayFingerprint::from_evidence_bytes(IDENTIFIED_DISPLAY_EVIDENCE);
        harness.write_current_primary_placement(display_fingerprint)?;
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
                .get::<WindowRestoreAttempt>(primary_window)
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
                digest: Digest::new(display_fingerprint.get()),
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
        let Some(attempt_id) = applying_attempt(&harness.app, &role) else {
            return Err("exact persisted identity did not start the window driver".into());
        };
        let attempt_ref = attempt_id;
        if !window_attempt_is_in_flight(&harness.app, &role, attempt_ref) {
            return Err("window driver attempt was not retained".into());
        }
        assert!(
            harness
                .app
                .world()
                .get::<WindowRestoreAttempt>(primary_window)
                .is_some()
        );
        assert_eq!(
            harness
                .app
                .world()
                .get::<WindowRestoreAttempt>(primary_window)
                .map(WindowRestoreAttempt::source),
            Some(RestorePreparationSource::KernelAttempt(attempt_ref))
        );
        let driver_state = harness.app.world().resource::<WindowRoleDriverState>();
        let RestoreRecord::UnderPreparation(preparation) = driver_state.restore_record(attempt_ref)
        else {
            return Err(String::from("window driver did not retain its preparation"));
        };
        assert_eq!(preparation.dispatched().logical_size, UVec2::new(640, 480));
        assert_eq!(
            preparation.dispatched().position,
            persistence::EstablishedWindowPosition::Restorable {
                logical_offset: IVec2::new(40, 30),
            }
        );

        Ok(StartedProductionWindowApply {
            primary_window,
            role,
            attempt: attempt_ref,
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
                .get::<WindowRestoreAttempt>(primary_window)
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
                .get::<WindowRestoreAttempt>(primary_window)
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

    fn applying_attempt(app: &App, role: &RoleKey) -> Option<AttemptRef> {
        match role_status_view(app, role)? {
            RoleStatusView::Applying { attempt, .. } => Some(*attempt),
            RoleStatusView::Waiting(_)
            | RoleStatusView::Established { .. }
            | RoleStatusView::Stopped(_) => None,
        }
    }

    /// Whether the window driver's ledger still runs the named attempt for that role.
    ///
    /// The ledger is the driver's only record now, so an attempt it does not name is one the
    /// driver holds nothing for — there is no second map left for a stale entry to hide in.
    fn window_attempt_is_in_flight(app: &App, role: &RoleKey, attempt: AttemptRef) -> bool {
        matches!(
            crate::driver::window_attempt_lookup(app.world(), role),
            AttemptLookup::Applying(in_flight) | AttemptLookup::CompletionQueued(in_flight)
                if in_flight == attempt
        )
    }

    /// Whether the window driver's ledger holds a session for that role, lost or not.
    ///
    /// A reported loss still leaves the driver holding the window its session established, which
    /// is why it counts here: the release that ends it has not arrived yet.
    fn window_session_is_established(app: &App, role: &RoleKey) -> bool {
        matches!(
            crate::driver::window_session_lookup(app.world(), role),
            SessionLookup::Holding(_) | SessionLookup::LossReported(_)
        )
    }

    /// Whether the window driver's ledger holds neither an attempt nor a session for that role.
    fn window_ledger_holds_nothing(app: &App, role: &RoleKey) -> bool {
        matches!(
            crate::driver::window_attempt_lookup(app.world(), role),
            AttemptLookup::Idle
        ) && matches!(
            crate::driver::window_session_lookup(app.world(), role),
            SessionLookup::NotEstablished
        )
    }

    fn role_status_view<'app>(app: &'app App, role: &RoleKey) -> Option<&'app RoleStatusView> {
        let entity = app.world().resource::<Bindings>().role_entity(role).ok()?;
        app.world().get::<RoleStatus>(entity).map(RoleStatus::view)
    }

    fn role_is_waiting(app: &App, role: &RoleKey) -> bool {
        matches!(
            role_status_view(app, role),
            Some(RoleStatusView::Waiting(_))
        )
    }

    fn role_is_established(app: &App, role: &RoleKey) -> bool {
        matches!(
            role_status_view(app, role),
            Some(RoleStatusView::Established { .. })
        )
    }

    fn device_for_identified_display(evidence: &[u8]) -> DeviceKey {
        let display_fingerprint = DisplayFingerprint::from_evidence_bytes(evidence);
        DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Synthesized {
                digest: Digest::new(display_fingerprint.get()),
            },
        }
    }

    fn set_primary_window_position(
        harness: &mut ProductionPluginHarness,
        primary_window: Entity,
        position: IVec2,
    ) -> Result<(), String> {
        {
            let mut window = harness
                .app
                .world_mut()
                .get_mut::<Window>(primary_window)
                .ok_or_else(|| String::from("primary window entity lost its Window"))?;
            window.position = WindowPosition::At(position);
        }
        Ok(())
    }

    fn set_primary_window_monitor(
        harness: &mut ProductionPluginHarness,
        primary_window: Entity,
        monitor: Entity,
        position: IVec2,
    ) -> Result<(), String> {
        set_primary_window_position(harness, primary_window, position)?;
        harness
            .app
            .world_mut()
            .entity_mut(primary_window)
            .insert(OnMonitor(monitor));
        Ok(())
    }

    fn primary_window_placement(
        harness: &ProductionPluginHarness,
        primary_window: Entity,
    ) -> Result<EstablishedWindowPlacement, String> {
        let world = harness.app.world();
        let window = world
            .get::<Window>(primary_window)
            .ok_or_else(|| String::from("primary window entity lost its Window"))?;
        let current_monitor = world
            .get::<CurrentMonitor>(primary_window)
            .ok_or_else(|| String::from("primary window entity lost its CurrentMonitor"))?;
        let physical_position = match window.position {
            WindowPosition::At(position) => Some(position),
            _ => None,
        };
        Ok(EstablishedWindowPlacement::from_readback(
            window,
            current_monitor,
            physical_position,
            *world.resource::<Platform>(),
        ))
    }

    fn finish_window_as_dispatched(
        harness: &mut ProductionPluginHarness,
        attempt: AttemptRef,
    ) -> Result<Entity, String> {
        let window = {
            let world = harness.app.world_mut();
            let mut attempts = world.query::<(Entity, &WindowRestoreAttempt)>();
            attempts
                .iter(world)
                .find_map(|(window, restore_attempt)| {
                    (restore_attempt.attempt() == attempt).then_some(window)
                })
                .ok_or_else(|| format!("window attempt {attempt:?} has no restore work"))?
        };
        harness
            .app
            .world_mut()
            .resource_mut::<WindowRoleDriverState>()
            .finish_as_dispatched(attempt);
        restore::remove_window_restore_work(harness.app.world_mut(), window, attempt);
        Ok(window)
    }

    fn finish_window_with_readback(
        harness: &mut ProductionPluginHarness,
        attempt: AttemptRef,
        placement: EstablishedWindowPlacement,
    ) -> Result<Entity, String> {
        let window = {
            let world = harness.app.world_mut();
            let mut attempts = world.query::<(Entity, &WindowRestoreAttempt)>();
            attempts
                .iter(world)
                .find_map(|(window, restore_attempt)| {
                    (restore_attempt.attempt() == attempt).then_some(window)
                })
                .ok_or_else(|| format!("window attempt {attempt:?} has no restore work"))?
        };
        harness
            .app
            .world_mut()
            .resource_mut::<WindowRoleDriverState>()
            .finish_with_readback(attempt, placement);
        restore::remove_window_restore_work(harness.app.world_mut(), window, attempt);
        Ok(window)
    }

    fn register_primary_window_binding(
        harness: &mut ProductionPluginHarness,
        device: DeviceKey,
    ) -> Result<(RoleKey, Entity), String> {
        let primary_window = harness.primary_window()?;
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let placement = primary_window_placement(harness, primary_window)?;
        let endpoint_id = PartName::new(role.as_str())
            .map(EndpointId::Part)
            .map_err(|error| format!("failed to create primary endpoint: {error}"))?;
        let binding = BindingAuthoring::new(
            role.clone(),
            DeviceEndpoint {
                device,
                id: endpoint_id,
            },
            harness.app.world().resource::<WindowDriverId>().0,
            placement,
            BindingPolicy::new(
                RecoveryPolicy::Forget,
                RetryOn::NewRevision,
                OnAbort::LeaveAsIs,
                OnSessionLoss::Recreate,
                ApplyDeadline::ProcessDefault,
            ),
        );
        let role_entity = register_binding(harness.app.world_mut(), binding)
            .map_err(|error| format!("failed to register primary binding: {error}"))?;
        harness.app.world_mut().entity_mut(primary_window).insert((
            managed::WindowRiggingRole::new(role_entity),
            WindowBindingAuthoring::Registered,
        ));
        Ok((role, role_entity))
    }

    fn establish_window_configuration(
        harness: &mut ProductionPluginHarness,
        role: &RoleKey,
    ) -> Result<(), String> {
        let mut attempt = None;
        for _ in 0..RECOVERY_TOPOLOGY_UPDATE_LIMIT {
            harness.app.update();
            if let Some(started) = applying_attempt(&harness.app, role) {
                attempt = Some(started);
                break;
            }
        }
        let attempt =
            attempt.ok_or_else(|| String::from("window role never started an initial apply"))?;
        finish_window_as_dispatched(harness, attempt)?;
        harness.app.update();
        harness.app.update();

        let binding = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(role)
            .map_err(|error| format!("window binding disappeared: {error}"))?;
        if binding.last_known_good().is_err() {
            return Err(String::from(
                "window role did not capture an established configuration",
            ));
        }
        Ok(())
    }

    fn wait_for_primary_waiting_work(
        harness: &mut ProductionPluginHarness,
        role: &RoleKey,
        waiting_work: WaitingWork,
    ) -> Result<(), String> {
        for _ in 0..RECOVERY_TOPOLOGY_UPDATE_LIMIT {
            if harness
                .app
                .world()
                .resource::<Bindings>()
                .waiting_work(role)
                == waiting_work
            {
                return Ok(());
            }
            harness.app.update();
        }
        let bindings = harness.app.world().resource::<Bindings>();
        let binding = bindings
            .binding(role)
            .map_err(|error| format!("primary window binding disappeared: {error}"))?;
        Err(format!(
            "primary role {role} did not reach departure work {waiting_work:?}; observed work {:?}, endpoint {:?}, and status {:?}",
            bindings.waiting_work(role),
            binding.endpoint.device,
            role_status_view(&harness.app, role)
        ))
    }

    fn primary_waiting_work(harness: &ProductionPluginHarness, role: &RoleKey) -> WaitingWork {
        harness
            .app
            .world()
            .resource::<Bindings>()
            .waiting_work(role)
    }

    fn wait_for_device_unavailability(
        harness: &mut ProductionPluginHarness,
        key: &DeviceKey,
    ) -> Result<(), String> {
        for _ in 0..RECOVERY_TOPOLOGY_UPDATE_LIMIT {
            harness.app.update();
            let world = harness.app.world_mut();
            let mut statuses = world.query::<&DeviceStatus>();
            if statuses.iter(world).any(|status| {
                status.key() == key && !matches!(status.availability(), KeyAvailability::Present(_))
            }) {
                return Ok(());
            }
        }
        Err(format!(
            "display device {key:?} did not publish an unavailable status"
        ))
    }

    fn stranded_baselines_are_tracked(harness: &ProductionPluginHarness, role: &RoleKey) -> bool {
        let world = harness.app.world();
        world
            .resource::<StrandedWindowMovementBaselines>()
            .is_tracked(role)
    }

    fn departed_primary_window(
        recovery_marker: PrimaryRecoveryMarkerState,
    ) -> Result<(ProductionPluginHarness, DepartedPrimaryWindow), String> {
        let mut harness = ProductionPluginHarness::with_window_plugin_and_window_manager(
            KernelInstallation::WindowManagerOwned,
            WindowPlugin::default(),
            |_| {},
            |path| recovery_marker.configure_window_manager(path),
        )?;
        let (departed_monitor, survivor_monitor) =
            harness.install_identified_headless_display_pair();
        let primary_window = harness.primary_window()?;
        harness
            .app
            .world_mut()
            .insert_resource(Monitors::from_test_monitors([
                (
                    departed_monitor,
                    MonitorDescriptor::for_current_enumeration(
                        0,
                        1.0,
                        IVec2::ZERO,
                        UVec2::new(1_920, 1_080),
                    ),
                ),
                (
                    survivor_monitor,
                    MonitorDescriptor::for_current_enumeration(
                        1,
                        1.0,
                        IVec2::new(1_920, 0),
                        UVec2::new(1_920, 1_080),
                    ),
                ),
            ]));
        set_primary_window_monitor(
            &mut harness,
            primary_window,
            departed_monitor,
            LEFT_DISPLAY_WINDOW_POSITION,
        )?;
        harness.app.update();
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        establish_window_configuration(&mut harness, &role)?;

        let departed_device = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&role)
            .map_err(|error| format!("primary window binding disappeared: {error}"))?
            .endpoint
            .device
            .clone();
        let survivor_device = device_for_identified_display(SECOND_IDENTIFIED_DISPLAY_EVIDENCE);
        harness.report_identified_display_departure(survivor_monitor)?;
        wait_for_device_unavailability(&mut harness, &departed_device)?;
        wait_for_primary_waiting_work(&mut harness, &role, recovery_marker.unavailability_work())?;
        set_primary_window_monitor(
            &mut harness,
            primary_window,
            survivor_monitor,
            RIGHT_DISPLAY_WINDOW_POSITION,
        )?;
        harness.app.world_mut().flush();
        if matches!(
            recovery_marker,
            PrimaryRecoveryMarkerState::RecoverOnRequest
                | PrimaryRecoveryMarkerState::RecoverOnReturn
        ) {
            for _ in 0..RECOVERY_TOPOLOGY_UPDATE_LIMIT {
                harness.app.update();
                if stranded_baselines_are_tracked(&harness, &role) {
                    break;
                }
            }
            if !stranded_baselines_are_tracked(&harness, &role) {
                return Err(format!(
                    "primary role {role} did not establish stranded-window baselines"
                ));
            }
        }
        harness.disconnect_identified_display(departed_monitor, survivor_monitor)?;

        Ok((
            harness,
            DepartedPrimaryWindow {
                primary_window,
                role,
                departed_device,
                survivor_device,
                survivor_monitor,
            },
        ))
    }

    fn despawn_primary_window(
        harness: &mut ProductionPluginHarness,
        primary_window: Entity,
    ) -> Result<(), String> {
        if harness.app.world_mut().despawn(primary_window) {
            Ok(())
        } else {
            Err(String::from("primary window did not despawn"))
        }
    }

    fn respawn_primary_window(harness: &mut ProductionPluginHarness, monitor: Entity) -> Entity {
        let mut window = Window {
            position: WindowPosition::At(RIGHT_DISPLAY_WINDOW_POSITION),
            ..default()
        };
        window.resolution.set(640.0, 480.0);
        let primary_window = harness
            .app
            .world_mut()
            .spawn((window, PrimaryWindow, OnMonitor(monitor)))
            .id();
        harness
            .app
            .world_mut()
            .init_resource::<InjectedWinitWindows>();
        harness
            .app
            .world_mut()
            .resource_mut::<InjectedWinitWindows>()
            .insert(primary_window, UVec2::ZERO);
        primary_window
    }

    fn wait_for_window_position(
        harness: &mut ProductionPluginHarness,
        primary_window: Entity,
        position: IVec2,
    ) -> Result<(), String> {
        for _ in 0..RECOVERY_TOPOLOGY_UPDATE_LIMIT {
            harness.app.update();
            if harness
                .app
                .world()
                .get::<Window>(primary_window)
                .is_some_and(|window| window.position == WindowPosition::At(position))
            {
                return Ok(());
            }
        }
        Err(format!(
            "primary window {primary_window} did not reach position {position}"
        ))
    }

    fn primary_binding_entity(
        harness: &ProductionPluginHarness,
        role: &RoleKey,
    ) -> Result<Entity, String> {
        harness
            .app
            .world()
            .resource::<Bindings>()
            .role_entity(role)
            .map_err(|_| format!("primary role {role} has no binding entity"))
    }

    fn wait_for_binding_endpoint(
        harness: &mut ProductionPluginHarness,
        role: &RoleKey,
        expected_device: &DeviceKey,
    ) -> Result<(), String> {
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            let device = harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(role)
                .map_err(|error| format!("window binding disappeared: {error}"))?
                .endpoint
                .device
                .clone();
            if device == *expected_device {
                return Ok(());
            }
            harness.app.update();
        }
        Err(format!(
            "window role {role} did not bind to expected display {expected_device:?}"
        ))
    }

    fn complete_successful_window_apply(
        harness: &mut ProductionPluginHarness,
        started: &StartedProductionWindowApply,
    ) -> Result<(), String> {
        finish_window_as_dispatched(harness, started.attempt)?;
        harness.app.update();

        assert!(!window_attempt_is_in_flight(
            &harness.app,
            &started.role,
            started.attempt
        ));
        let binding = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&started.role)
            .map_err(|error| format!("settled window binding disappeared: {error}"))?;
        assert!(role_is_established(&harness.app, &started.role));
        assert!(matches!(
            binding.last_known_good,
            LastKnownGoodConfiguration::MatchesRequested
        ));
        Ok(())
    }

    /// Losing the primary marker takes the aborted attempt's restore work off the live window.
    ///
    /// `on_primary_window_removed` fires on `Remove<PrimaryWindow>`, which is not a despawn: the
    /// window entity is still there afterwards, and in the case the observer is written for — a
    /// registered name keeping it managed — it stays a window Clerestory places. `abort_window`
    /// ends the ledger's claim, but the ledger cannot reach the world, so the markers the attempt
    /// left have to be stripped by the caller. A stale `TargetPosition` is the costly one:
    /// `prepare_driver_restore_targets` queries `Without<TargetPosition>` and can never build that
    /// window a target again.
    #[test]
    fn primary_marker_removal_strips_the_aborted_attempt_s_restore_work() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        let window = started.primary_window;
        let monitor = harness
            .app
            .world()
            .resource::<Monitors>()
            .iter()
            .find(|live| {
                harness
                    .app
                    .world()
                    .get::<CurrentMonitor>(window)
                    .is_some_and(|current| current.descriptor == *live.descriptor)
            })
            .map(|live| live.entity)
            .ok_or_else(|| {
                String::from("the applying window sits on no monitor the topology reports")
            })?;
        // The harness's primary window receives neither `OnMonitor` nor a native window, and
        // `prepare_driver_restore_targets` withholds `TargetPosition` without both. Supplying them
        // is what gives the abort something to leave behind.
        harness
            .app
            .world_mut()
            .entity_mut(window)
            .insert(OnMonitor(monitor));
        harness
            .app
            .world_mut()
            .init_resource::<InjectedWinitWindows>();
        harness
            .app
            .world_mut()
            .resource_mut::<InjectedWinitWindows>()
            .insert(window, UVec2::ZERO);
        harness.app.update();
        assert!(
            harness.app.world().get::<TargetPosition>(window).is_some(),
            "the fixture never built a target for the attempt, so removing one proves nothing"
        );

        harness
            .app
            .world_mut()
            .entity_mut(window)
            .remove::<PrimaryWindow>();
        harness.app.world_mut().flush();

        assert!(
            harness.app.world().get_entity(window).is_ok(),
            "losing the primary marker despawned the window, so nothing could be left standing"
        );
        assert!(
            harness
                .app
                .world()
                .get::<WindowRestoreAttempt>(window)
                .is_none(),
            "the aborted attempt's restore marker is still on the live window"
        );
        assert!(
            harness.app.world().get::<TargetPosition>(window).is_none(),
            "the aborted attempt's target is still on the live window, so \
             prepare_driver_restore_targets can never re-target it"
        );
        assert!(window_ledger_holds_nothing(&harness.app, &started.role));
        Ok(())
    }

    /// A window role the kernel has dispatched an attempt for, still applying.
    ///
    /// `attempt` is a reference the kernel really issued, which is the only kind that exists:
    /// `AttemptRef` has no constructor outside `hana_rigging`, so a driver test that needs one
    /// starts here rather than fabricating it.
    pub(crate) struct ApplyingWindowRole {
        pub(crate) app:     App,
        pub(crate) role:    RoleKey,
        pub(crate) window:  Entity,
        pub(crate) attempt: AttemptRef,
        _directory:         TempDir,
    }

    /// Dispatch one window placement attempt through the production plugin set and stop there.
    ///
    /// The attempt is left applying: the driver holds its preparation, the window carries
    /// `WindowRestoreAttempt`, and the ledger's slot for the role names it. `TargetPosition` is
    /// not there: the harness's primary window never receives `OnMonitor` and has no native
    /// window behind it, so a test that needs the position marker supplies both itself.
    pub(crate) fn applying_window_role() -> Result<ApplyingWindowRole, String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        let ProductionPluginHarness { app, directory } = harness;
        Ok(ApplyingWindowRole {
            app,
            role: started.role,
            window: started.primary_window,
            attempt: started.attempt,
            _directory: directory,
        })
    }

    /// A window role whose kernel session was replaced after a first session was established.
    ///
    /// Holds both session references: `stale_session` is the one the role no longer holds, and
    /// `live_session` is the successor's. A driver test hands the stale one back through
    /// `release_session` to see what the driver does with a window that is not that session's.
    pub(crate) struct ReEstablishedWindowRole {
        pub(crate) app:           App,
        pub(crate) role:          RoleKey,
        pub(crate) role_entity:   Entity,
        pub(crate) window:        Entity,
        pub(crate) stale_session: SessionRef,
        pub(crate) live_session:  SessionRef,
        _directory:               TempDir,
    }

    /// Establish a window session, replace its binding, and establish the successor's session.
    ///
    /// The production plugin set throughout: the kernel issues both sessions and the window
    /// driver files both leases, so the stale reference the fixture hands back is one the kernel
    /// really issued and really replaced.
    pub(crate) fn re_established_window_role() -> Result<ReEstablishedWindowRole, String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        complete_successful_window_apply(&mut harness, &started)?;
        let SessionLookup::Holding(stale_session) =
            crate::driver::window_session_lookup(harness.app.world(), &started.role)
        else {
            return Err(String::from(
                "the first window apply established no session",
            ));
        };

        let placement = primary_window_placement(&harness, started.primary_window)?;
        let established_endpoint = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&started.role)
            .map_err(|error| format!("established window binding disappeared: {error}"))?
            .endpoint
            .clone();
        let replacement = BindingAuthoring::new(
            started.role.clone(),
            DeviceEndpoint {
                device: established_endpoint.device,
                id:     EndpointId::Part(
                    PartName::new("re-established-window")
                        .map_err(|error| format!("failed to create rebound endpoint: {error}"))?,
                ),
            },
            harness.app.world().resource::<WindowDriverId>().0,
            placement,
            BindingPolicy::new(
                RecoveryPolicy::Forget,
                RetryOn::NewRevision,
                OnAbort::LeaveAsIs,
                OnSessionLoss::Recreate,
                ApplyDeadline::ProcessDefault,
            ),
        );
        replace_binding(harness.app.world_mut(), replacement).map_err(|error| {
            format!("failed to replace the established window binding: {error}")
        })?;

        let mut successor = None;
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            successor = applying_attempt(&harness.app, &started.role);
            if successor.is_some() {
                break;
            }
        }
        let successor = successor
            .ok_or_else(|| String::from("the replacement dispatched no successor attempt"))?;
        finish_window_as_dispatched(&mut harness, successor)?;

        let mut live = SessionLookup::NotEstablished;
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            live = crate::driver::window_session_lookup(harness.app.world(), &started.role);
            if matches!(live, SessionLookup::Holding(_)) {
                break;
            }
        }
        let SessionLookup::Holding(live_session) = live else {
            return Err(String::from("the successor attempt established no session"));
        };
        if live_session == stale_session {
            return Err(String::from(
                "the successor reused the predecessor's session reference",
            ));
        }

        let role_entity = harness
            .app
            .world()
            .resource::<Bindings>()
            .role_entity(&started.role)
            .map_err(|error| format!("re-established role has no entity: {error}"))?;
        let ProductionPluginHarness { app, directory } = harness;
        Ok(ReEstablishedWindowRole {
            app,
            role: started.role,
            role_entity,
            window: started.primary_window,
            stale_session,
            live_session,
            _directory: directory,
        })
    }

    fn assert_exact_completion_does_not_report_drift(
        harness: &mut ProductionPluginHarness,
        started: &StartedProductionWindowApply,
    ) -> Result<(), String> {
        harness.app.update();
        harness.app.update();

        let binding = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&started.role)
            .map_err(|error| format!("settled window binding disappeared: {error}"))?;
        assert!(matches!(
            binding.last_known_good,
            LastKnownGoodConfiguration::MatchesRequested
        ));
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
        register_binding(harness.app.world_mut(), binding)
            .map_err(|error| format!("failed to register managed role: {error}"))?;
        let entity = harness
            .app
            .world_mut()
            .spawn(ManagedWindowName("inspector".into()))
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
    fn configured_primary_recovery_authors_the_selected_policy() -> Result<(), String> {
        let mut return_harness = ProductionPluginHarness::with_window_plugin_and_window_manager(
            KernelInstallation::WindowManagerOwned,
            WindowPlugin::default(),
            |_| {},
            |path| WindowManagerPlugin::with_path(path).recover_on_return(),
        )?;
        return_harness.install_identified_headless_window_topology();
        for _ in 0..3 {
            return_harness.app.update();
        }
        assert_eq!(
            return_harness.primary_recovery_policy()?,
            RecoveryPolicy::ReapplyOnReturn
        );

        let mut request_harness = ProductionPluginHarness::with_window_plugin_and_window_manager(
            KernelInstallation::WindowManagerOwned,
            WindowPlugin::default(),
            |_| {},
            |path| WindowManagerPlugin::with_path(path).recover_on_request(),
        )?;
        request_harness.install_identified_headless_window_topology();
        for _ in 0..3 {
            request_harness.app.update();
        }
        assert_eq!(
            request_harness.primary_recovery_policy()?,
            RecoveryPolicy::ReapplyOnRequest
        );

        let mut restore_harness =
            ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        restore_harness.install_identified_headless_window_topology();
        for _ in 0..3 {
            restore_harness.app.update();
        }
        assert_eq!(
            restore_harness.primary_recovery_policy()?,
            RecoveryPolicy::Forget
        );
        Ok(())
    }

    #[test]
    fn configured_primary_recovery_marks_a_respawned_primary() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::with_window_plugin_and_window_manager(
            KernelInstallation::WindowManagerOwned,
            WindowPlugin::default(),
            |_| {},
            |path| WindowManagerPlugin::with_path(path).recover_on_return(),
        )?;
        let original_primary_window = harness.primary_window()?;

        assert!(harness.app.world_mut().despawn(original_primary_window));
        harness.app.world_mut().flush();
        let respawned_primary_window = harness
            .app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        harness.app.world_mut().flush();

        assert!(
            harness
                .app
                .world()
                .get::<RecoverOnReturn>(respawned_primary_window)
                .is_some()
        );
        Ok(())
    }

    #[test]
    fn an_existing_primary_and_its_respawn_are_each_hidden_with_a_fresh_wait() {
        let mut app = App::new();
        let original = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();

        hide_startup_window(&mut app, Platform::MacOs);

        assert_eq!(
            app.world()
                .get::<Window>(original)
                .map(|window| window.visible),
            Some(false)
        );
        assert!(
            app.world()
                .get::<visibility::SavedDisplayRevealWait>(original)
                .is_some()
        );

        assert!(app.world_mut().despawn(original));
        let respawned = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        app.world_mut().flush();

        assert_eq!(
            app.world()
                .get::<Window>(respawned)
                .map(|window| window.visible),
            Some(false)
        );
        assert!(
            app.world()
                .get::<visibility::SavedDisplayRevealWait>(respawned)
                .is_some()
        );
    }

    #[test]
    fn configured_primary_recovery_backfills_an_existing_primary() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::with_window_plugin_and_window_manager(
            KernelInstallation::WindowManagerOwned,
            WindowPlugin::default(),
            |_| {},
            |path| WindowManagerPlugin::with_path(path).recover_on_return(),
        )?;
        let primary_window = harness.primary_window()?;

        assert!(
            harness
                .app
                .world()
                .get::<RecoverOnReturn>(primary_window)
                .is_some()
        );
        Ok(())
    }

    #[test]
    fn configured_primary_recovery_warns_once_for_an_opposite_backfill_marker() -> Result<(), String>
    {
        let mut harness = ProductionPluginHarness::with_window_plugin_and_window_manager(
            KernelInstallation::WindowManagerOwned,
            WindowPlugin {
                primary_window: None,
                ..default()
            },
            |app| {
                app.init_resource::<PrimaryRecoveryMarkerWarnings>()
                    .add_observer(record_primary_recovery_marker_warning);
                app.world_mut()
                    .spawn((Window::default(), PrimaryWindow, RecoverOnRequest));
            },
            |path| WindowManagerPlugin::with_path(path).recover_on_return(),
        )?;
        let primary_window = harness.primary_window()?;
        harness.install_identified_headless_window_topology();
        for _ in 0..3 {
            harness.app.update();
        }

        assert_eq!(
            harness.primary_recovery_policy()?,
            RecoveryPolicy::ReapplyOnRequest
        );
        assert_eq!(
            harness
                .app
                .world()
                .resource::<PrimaryRecoveryMarkerWarnings>()
                .0,
            1
        );
        assert!(
            harness
                .app
                .world()
                .get::<managed::RecoveryMarkerConflictWarned>(primary_window)
                .is_some()
        );
        Ok(())
    }

    #[test]
    fn configured_primary_recovery_uses_the_last_builder_call() -> Result<(), String> {
        let mut return_harness = ProductionPluginHarness::with_window_plugin_and_window_manager(
            KernelInstallation::WindowManagerOwned,
            WindowPlugin::default(),
            |_| {},
            |path| {
                WindowManagerPlugin::with_path(path)
                    .recover_on_request()
                    .recover_on_return()
            },
        )?;
        let return_primary_window = return_harness.primary_window()?;
        assert!(
            return_harness
                .app
                .world()
                .get::<RecoverOnReturn>(return_primary_window)
                .is_some()
        );
        assert!(
            return_harness
                .app
                .world()
                .get::<RecoverOnRequest>(return_primary_window)
                .is_none()
        );

        let mut request_harness = ProductionPluginHarness::with_window_plugin_and_window_manager(
            KernelInstallation::WindowManagerOwned,
            WindowPlugin::default(),
            |_| {},
            |path| {
                WindowManagerPlugin::with_path(path)
                    .recover_on_return()
                    .recover_on_request()
            },
        )?;
        let request_primary_window = request_harness.primary_window()?;
        assert!(
            request_harness
                .app
                .world()
                .get::<RecoverOnRequest>(request_primary_window)
                .is_some()
        );
        assert!(
            request_harness
                .app
                .world()
                .get::<RecoverOnReturn>(request_primary_window)
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn current_exact_identity_applies_without_reporting_immediate_drift() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        complete_successful_window_apply(&mut harness, &started)?;
        assert_exact_completion_does_not_report_drift(&mut harness, &started)
    }

    #[test]
    fn role_without_window_relationship_waits_without_receiving_an_attempt() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let primary_window = harness.primary_window()?;
        harness.app.add_systems(
            Update,
            remove_primary_window_role_before_resolution
                .after(ClerestoryWindowDriverSet::BindingAuthoring)
                .before(RiggingSystems::Apply),
        );
        harness.install_identified_headless_window_topology();
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;

        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            if applying_attempt(&harness.app, &role).is_some()
                || !crate::driver::window_ledger_is_empty(harness.app.world())
            {
                return Err(String::from(
                    "the window driver's ledger took an attempt, under any role, while the \
                     window role had no role relationship",
                ));
            }
            let Ok(role_entity) = harness
                .app
                .world()
                .resource::<Bindings>()
                .role_entity(&role)
            else {
                continue;
            };
            let waiting_for_attachment = harness
                .app
                .world()
                .get::<RoleStatus>(role_entity)
                .is_some_and(|status| {
                    matches!(
                        status.view(),
                        RoleStatusView::Waiting(
                            WaitingStatusView::ApplicationTargetAttachmentRequired { .. }
                        )
                    )
                });
            if waiting_for_attachment {
                assert!(
                    harness
                        .app
                        .world()
                        .get::<managed::WindowRiggingRole>(primary_window)
                        .is_none()
                );
                assert!(
                    harness
                        .app
                        .world()
                        .get::<WindowRestoreAttempt>(primary_window)
                        .is_none()
                );
                return Ok(());
            }
        }
        Err(format!(
            "window role {role} did not wait for its application attachment"
        ))
    }

    #[test]
    fn completed_window_that_ends_before_establishment_reports_session_loss() -> Result<(), String>
    {
        let mut harness = ProductionPluginHarness::with_window_plugin_and_window_manager(
            KernelInstallation::WindowManagerOwned,
            WindowPlugin::default(),
            |_| {},
            |path| WindowManagerPlugin::with_path(path).recover_on_request(),
        )?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        let recovery = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&started.role)
            .map_err(|error| format!("started window binding disappeared: {error}"))?
            .recovery;
        assert_eq!(recovery, RecoveryPolicy::ReapplyOnRequest);

        finish_window_as_dispatched(&mut harness, started.attempt)?;
        despawn_primary_window(&mut harness, started.primary_window)?;
        assert!(window_attempt_is_in_flight(
            &harness.app,
            &started.role,
            started.attempt
        ));

        harness.app.update();
        harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&started.role)
            .map_err(|error| format!("accepted window binding disappeared: {error}"))?;
        assert!(role_is_established(&harness.app, &started.role));
        assert!(!window_session_is_established(&harness.app, &started.role));

        harness.app.update();
        let bindings = harness.app.world().resource::<Bindings>();
        bindings
            .binding(&started.role)
            .map_err(|error| format!("reported-loss window binding disappeared: {error}"))?;
        assert!(role_is_waiting(&harness.app, &started.role));
        assert!(window_ledger_holds_nothing(&harness.app, &started.role));
        Ok(())
    }

    #[test]
    fn departed_target_monitor_fails_preparation_and_marks_the_role_missing() -> Result<(), String>
    {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        let fallback_monitor = harness.app.world_mut().spawn_empty().id();
        let fallback_descriptor = MonitorDescriptor::for_current_enumeration(
            0,
            1.0,
            IVec2::new(1_920, 0),
            UVec2::new(1_920, 1_080),
        );
        prepare_window_target_with_monitors(
            &mut harness,
            started.primary_window,
            fallback_monitor,
            fallback_descriptor,
            Monitors::from_test_monitors([(fallback_monitor, fallback_descriptor)]),
        )?;

        assert!(!window_attempt_is_in_flight(
            &harness.app,
            &started.role,
            started.attempt
        ));
        assert!(
            harness
                .app
                .world()
                .get::<WindowRestoreAttempt>(started.primary_window)
                .is_none()
        );
        assert_eq!(
            harness
                .app
                .world()
                .resource::<recovery::WindowFallbackRecoveryState>()
                .phase(&started.role),
            WindowFallbackRecoveryProgress::Recovering(
                WindowFallbackRecoveryPhase::MissingLiveMonitor
            )
        );
        Ok(())
    }

    #[test]
    fn live_target_with_changed_descriptor_uses_current_geometry() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        let RestoreRecord::UnderPreparation(preparation) = harness
            .app
            .world()
            .resource::<WindowRoleDriverState>()
            .restore_record(started.attempt)
        else {
            return Err(String::from("window driver lost its target preparation"));
        };
        let target_monitor = preparation.target().monitor;
        let current_descriptor = MonitorDescriptor::for_current_enumeration(
            3,
            2.0,
            IVec2::new(1_920, 0),
            UVec2::new(1_920, 1_080),
        );
        harness.app.insert_resource(Platform::Windows);
        prepare_window_target_with_monitors(
            &mut harness,
            started.primary_window,
            target_monitor,
            current_descriptor,
            Monitors::from_test_monitors([(target_monitor, current_descriptor)]),
        )?;

        assert!(window_attempt_is_in_flight(
            &harness.app,
            &started.role,
            started.attempt
        ));
        assert!(
            harness
                .app
                .world()
                .get::<TargetPosition>(started.primary_window)
                .is_some()
        );
        assert_eq!(
            harness
                .app
                .world()
                .resource::<recovery::WindowFallbackRecoveryState>()
                .phase(&started.role),
            WindowFallbackRecoveryProgress::NotRecovering
        );
        harness
            .app
            .world_mut()
            .run_system_once(restore::place_window_at_saved_geometry)
            .map_err(|error| format!("failed to apply the prepared window target: {error}"))?;
        let expected_position = current_descriptor.physical_from_logical_offset(IVec2::new(40, 30));
        assert_eq!(
            harness
                .app
                .world()
                .get::<Window>(started.primary_window)
                .map(|window| window.position),
            Some(WindowPosition::At(expected_position))
        );
        Ok(())
    }

    #[test]
    fn missing_live_display_component_registration_names_the_capability_type() -> Result<(), String>
    {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        harness
            .app
            .world()
            .resource::<AppTypeRegistry>()
            .write()
            .overwrite_registration(TypeRegistration::of::<LiveDisplayEndpoint>());
        harness.install_identified_headless_window_topology();
        harness.app.update();
        let device = device_for_identified_display(IDENTIFIED_DISPLAY_EVIDENCE);
        let (role, role_entity) = register_primary_window_binding(&mut harness, device)?;

        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            let waiting_for_registration = harness
                .app
                .world()
                .get::<RoleStatus>(role_entity)
                .is_some_and(|status| {
                    matches!(
                        status.view(),
                        RoleStatusView::Waiting(
                            WaitingStatusView::ApplicationCapabilityRegistrationRequired {
                                failure:
                                    CapabilityProjectionFailure::ReflectComponentNotRegistered {
                                        type_path,
                                    },
                                ..
                            }
                        ) if type_path == LiveDisplayEndpoint::type_path()
                    )
                });
            if waiting_for_registration {
                return Ok(());
            }
        }
        Err(format!(
            "window role {role} did not name the missing live display component registration"
        ))
    }

    #[test]
    fn live_display_endpoint_not_yet_reported_produces_an_ordinary_reporter_wait()
    -> Result<(), String> {
        let mut harness =
            ProductionPluginHarness::without_primary(KernelInstallation::WindowManagerOwned)?;
        let monitor = harness.install_identified_headless_window_topology();
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            if harness
                .app
                .world()
                .get::<LiveDisplayDevices>(monitor)
                .is_some()
            {
                break;
            }
        }
        harness.app.add_systems(
            Update,
            remove_live_display_before_window_resolution
                .after(RiggingSystems::Reconcile)
                .before(ClerestoryWindowDriverSet::BindingAuthoring),
        );
        harness
            .app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow, OnMonitor(monitor)));
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;

        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            let Ok(role_entity) = harness
                .app
                .world()
                .resource::<Bindings>()
                .role_entity(&role)
            else {
                continue;
            };
            let waiting_for_reporter = harness
                .app
                .world()
                .get::<RoleStatus>(role_entity)
                .is_some_and(|status| {
                    matches!(
                        status.view(),
                        RoleStatusView::Waiting(WaitingStatusView::Reporter(_))
                    )
                });
            if waiting_for_reporter {
                return Ok(());
            }
        }
        Err(format!(
            "window role {role} did not wait for the display reporter"
        ))
    }

    #[test]
    fn substituted_window_placement_is_recorded_as_different_from_the_request() -> Result<(), String>
    {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        let applied = primary_window_placement(&harness, started.primary_window)?;
        finish_window_with_readback(&mut harness, started.attempt, applied.clone())?;
        harness.app.update();

        let binding = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&started.role)
            .map_err(|error| format!("settled window binding disappeared: {error}"))?;
        let LastKnownGoodConfiguration::DiffersFromDispatched(configuration) =
            &binding.last_known_good
        else {
            return Err(String::from(
                "substituted placement was recorded as the dispatched request",
            ));
        };
        assert_eq!(
            configuration
                .as_any()
                .downcast_ref::<EstablishedWindowPlacement>(),
            Some(&applied)
        );
        Ok(())
    }

    #[test]
    fn replacement_registration_cancels_in_flight_completion_and_new_attempt_establishes()
    -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        let placement = primary_window_placement(&harness, started.primary_window)?;
        let current_endpoint = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&started.role)
            .map_err(|error| format!("started window binding disappeared: {error}"))?
            .endpoint
            .clone();
        let rebound_endpoint = DeviceEndpoint {
            device: current_endpoint.device,
            id:     EndpointId::Part(
                PartName::new("rebound-window")
                    .map_err(|error| format!("failed to create rebound endpoint: {error}"))?,
            ),
        };
        let replacement = BindingAuthoring::new(
            started.role.clone(),
            rebound_endpoint.clone(),
            harness.app.world().resource::<WindowDriverId>().0,
            placement,
            BindingPolicy::new(
                RecoveryPolicy::Forget,
                RetryOn::NewRevision,
                OnAbort::LeaveAsIs,
                OnSessionLoss::Recreate,
                ApplyDeadline::ProcessDefault,
            ),
        );
        let _ = replace_binding(harness.app.world_mut(), replacement)
            .map_err(|error| format!("failed to replace window binding: {error}"))?;

        harness.app.update();

        assert!(!window_attempt_is_in_flight(
            &harness.app,
            &started.role,
            started.attempt
        ));
        let binding = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&started.role)
            .map_err(|error| format!("rebound window binding disappeared: {error}"))?;
        assert_eq!(binding.endpoint, rebound_endpoint);
        let Some(rebound_attempt) = applying_attempt(&harness.app, &started.role) else {
            return Err("rebound window role did not dispatch a recoverable attempt".into());
        };
        assert_ne!(rebound_attempt, started.attempt);
        let preparation = harness
            .app
            .world()
            .get::<WindowRestoreAttempt>(started.primary_window)
            .ok_or_else(|| String::from("rebound window attempt did not prepare its target"))?;
        assert_eq!(
            preparation.source(),
            RestorePreparationSource::KernelAttempt(rebound_attempt)
        );
        assert!(matches!(
            harness
                .app
                .world()
                .resource::<WindowRoleDriverState>()
                .restore_record(rebound_attempt),
            RestoreRecord::UnderPreparation(_)
        ));
        finish_window_as_dispatched(&mut harness, rebound_attempt)?;
        harness.app.update();
        harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&started.role)
            .map_err(|error| format!("established replacement disappeared: {error}"))?;
        assert!(role_is_established(&harness.app, &started.role));
        assert!(window_session_is_established(&harness.app, &started.role));

        Ok(())
    }

    #[test]
    fn replacement_after_finish_before_drain_cleans_queued_work_and_establishes_the_successor()
    -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        let placement = primary_window_placement(&harness, started.primary_window)?;
        let current_endpoint = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&started.role)
            .map_err(|error| format!("started window binding disappeared: {error}"))?
            .endpoint
            .clone();
        let rebound_endpoint = DeviceEndpoint {
            device: current_endpoint.device,
            id:     EndpointId::Part(
                PartName::new("queued-rebound-window")
                    .map_err(|error| format!("failed to create rebound endpoint: {error}"))?,
            ),
        };
        finish_window_as_dispatched(&mut harness, started.attempt)?;
        assert!(window_attempt_is_in_flight(
            &harness.app,
            &started.role,
            started.attempt
        ));
        let replacement = BindingAuthoring::new(
            started.role.clone(),
            rebound_endpoint,
            harness.app.world().resource::<WindowDriverId>().0,
            placement,
            BindingPolicy::new(
                RecoveryPolicy::Forget,
                RetryOn::NewRevision,
                OnAbort::LeaveAsIs,
                OnSessionLoss::Recreate,
                ApplyDeadline::ProcessDefault,
            ),
        );
        replace_binding(harness.app.world_mut(), replacement)
            .map_err(|error| format!("failed to replace queued window binding: {error}"))?;

        harness.app.update();

        assert!(!window_attempt_is_in_flight(
            &harness.app,
            &started.role,
            started.attempt
        ));
        let successor = applying_attempt(&harness.app, &started.role)
            .ok_or_else(|| String::from("replacement did not dispatch its successor"))?;
        assert_ne!(successor, started.attempt);
        finish_window_as_dispatched(&mut harness, successor)?;
        harness.app.update();
        assert!(window_session_is_established(&harness.app, &started.role));
        Ok(())
    }

    #[test]
    fn retirement_releases_window_session_before_role_entity_despawns() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        complete_successful_window_apply(&mut harness, &started)?;
        let role_entity = harness
            .app
            .world()
            .resource::<Bindings>()
            .role_entity(&started.role)
            .map_err(|error| format!("established role has no entity: {error}"))?;
        assert!(window_session_is_established(&harness.app, &started.role));

        despawn_primary_window(&mut harness, started.primary_window)?;
        harness.app.update();

        assert!(!window_session_is_established(&harness.app, &started.role));
        assert!(harness.app.world().get_entity(role_entity).is_err());
        Ok(())
    }

    #[test]
    fn retirement_after_finish_before_drain_cleans_queued_window_work() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        finish_window_as_dispatched(&mut harness, started.attempt)?;
        assert!(window_attempt_is_in_flight(
            &harness.app,
            &started.role,
            started.attempt
        ));

        despawn_primary_window(&mut harness, started.primary_window)?;
        harness.app.update();

        assert!(!window_attempt_is_in_flight(
            &harness.app,
            &started.role,
            started.attempt
        ));
        assert!(
            harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&started.role)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn completion_after_hard_end_is_refused_and_role_shows_overrun_exhausted() -> Result<(), String>
    {
        let mut harness = ProductionPluginHarness::with_short_apply_bounds()?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        let role_entity = harness
            .app
            .world()
            .resource::<Bindings>()
            .role_entity(&started.role)
            .map_err(|error| format!("applying role has no entity: {error}"))?;
        harness
            .app
            .insert_resource(ScheduledWindowCompletion {
                attempt: started.attempt,
                window:  started.primary_window,
            })
            .add_systems(
                Update,
                finish_scheduled_window_completion
                    .after(RiggingSystems::Collect)
                    .before(RiggingSystems::Apply),
            )
            .insert_resource(TimeUpdateStrategy::ManualDuration(
                TEST_APPLY_DEADLINE + TEST_APPLY_OVERRUN + Duration::from_nanos(1),
            ));

        harness.app.update();

        assert_eq!(
            harness.app.world().get::<AttemptEnding>(role_entity),
            Some(&AttemptEnding::Invalidated(
                AttemptInvalidation::OverrunExhausted,
            ))
        );
        assert!(!window_attempt_is_in_flight(
            &harness.app,
            &started.role,
            started.attempt
        ));
        Ok(())
    }

    #[test]
    fn completion_finished_at_deadline_and_drained_during_overrun_establishes() -> Result<(), String>
    {
        let mut harness = ProductionPluginHarness::with_short_apply_bounds()?;
        let started = start_current_exact_identity_apply(&mut harness)?;
        harness
            .app
            .insert_resource(TimeUpdateStrategy::ManualDuration(TEST_APPLY_DEADLINE));
        harness.app.update();
        finish_window_as_dispatched(&mut harness, started.attempt)?;
        harness
            .app
            .insert_resource(TimeUpdateStrategy::ManualDuration(TEST_APPLY_OVERRUN / 2));

        harness.app.update();

        harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&started.role)
            .map_err(|error| format!("delayed completion lost its role: {error}"))?;
        assert!(role_is_established(&harness.app, &started.role));
        assert!(window_session_is_established(&harness.app, &started.role));
        Ok(())
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

    /// A v1 or v2 file must still restore, which requires a `Binding` to exist for its role.
    ///
    /// `device_for_legacy_display` compares a saved fingerprint against a live one, so the
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
        let display_fingerprint =
            monitors::DisplayFingerprint::from_evidence_bytes(IDENTIFIED_DISPLAY_EVIDENCE);
        let expected_device = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Synthesized {
                digest: Digest::new(display_fingerprint.get()),
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
    /// and `project_established` returns `AbsentBinding` for a role with no binding. The two halves
    /// therefore fail together, so the restore assertion above cannot stand in for this one.
    #[test]
    fn a_restored_anonymous_legacy_placement_is_rewritten_as_a_classified_record()
    -> Result<(), String> {
        let mut harness = ProductionPluginHarness::new(KernelInstallation::WindowManagerOwned)?;
        harness.write_legacy_anonymous_primary_placement()?;
        harness.install_identified_headless_window_topology();
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;

        let mut attempt = None;
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            if let Some(started) = applying_attempt(&harness.app, &role) {
                attempt = Some(started);
                break;
            }
        }
        let attempt = attempt
            .ok_or_else(|| String::from("anonymous legacy placement never started a restore"))?;

        finish_window_as_dispatched(&mut harness, attempt)?;
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

    #[test]
    fn departed_primary_windows_preserve_opted_in_endpoints_and_adopt_the_survivor()
    -> Result<(), String> {
        for recovery_marker in [
            PrimaryRecoveryMarkerState::Absent,
            PrimaryRecoveryMarkerState::RecoverOnRequest,
            PrimaryRecoveryMarkerState::RecoverOnReturn,
        ] {
            let (mut harness, departed) = departed_primary_window(recovery_marker)?;
            let (recovery, endpoint_device) = {
                let binding = harness
                    .app
                    .world()
                    .resource::<Bindings>()
                    .binding(&departed.role)
                    .map_err(|error| format!("primary window binding disappeared: {error}"))?;
                (binding.recovery, binding.endpoint.device.clone())
            };
            assert_eq!(recovery, recovery_marker.recovery_policy());
            assert_eq!(
                harness
                    .app
                    .world()
                    .resource::<Bindings>()
                    .waiting_work(&departed.role),
                recovery_marker.unavailability_work()
            );
            match recovery_marker {
                PrimaryRecoveryMarkerState::Absent => {
                    wait_for_binding_endpoint(
                        &mut harness,
                        &departed.role,
                        &departed.survivor_device,
                    )?;
                    assert!(
                        !harness
                            .app
                            .world()
                            .resource::<recovery::StrandedWindowMovementBaselines>()
                            .is_tracked(&departed.role)
                    );
                },
                PrimaryRecoveryMarkerState::RecoverOnRequest
                | PrimaryRecoveryMarkerState::RecoverOnReturn => {
                    assert_eq!(endpoint_device, departed.departed_device);
                    assert!(
                        harness
                            .app
                            .world()
                            .resource::<recovery::StrandedWindowMovementBaselines>()
                            .is_tracked(&departed.role)
                    );
                },
            }
        }
        Ok(())
    }

    #[test]
    fn display_driven_window_loss_respawns_across_all_recovery_marker_states() -> Result<(), String>
    {
        for recovery_marker in [
            PrimaryRecoveryMarkerState::Absent,
            PrimaryRecoveryMarkerState::RecoverOnRequest,
            PrimaryRecoveryMarkerState::RecoverOnReturn,
        ] {
            let (mut harness, departed) = departed_primary_window(recovery_marker)?;
            despawn_primary_window(&mut harness, departed.primary_window)?;
            harness.app.update();

            let retained = harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&departed.role);
            match recovery_marker {
                PrimaryRecoveryMarkerState::Absent => assert!(retained.is_err()),
                PrimaryRecoveryMarkerState::RecoverOnRequest
                | PrimaryRecoveryMarkerState::RecoverOnReturn => {
                    let binding = retained.map_err(|error| {
                        format!("opted-in primary binding retired on window loss: {error}")
                    })?;
                    assert_eq!(binding.endpoint.device, departed.departed_device);
                    assert_eq!(
                        primary_waiting_work(&harness, &departed.role),
                        recovery_marker.departure_work()
                    );
                },
            }

            let respawned = respawn_primary_window(&mut harness, departed.survivor_monitor);
            harness.app.update();
            harness.app.update();
            let (recovery, endpoint_device) = {
                let bindings = harness.app.world().resource::<Bindings>();
                let binding = bindings.binding(&departed.role).map_err(|error| {
                    format!("respawned primary window did not reattach: {error}")
                })?;
                (binding.recovery, binding.endpoint.device.clone())
            };
            assert_eq!(recovery, recovery_marker.recovery_policy());
            assert_eq!(
                primary_waiting_work(&harness, &departed.role),
                recovery_marker.departure_work()
            );
            assert_eq!(endpoint_device, departed.departed_device);

            harness.reconnect_identified_left_display(departed.survivor_monitor);
            match recovery_marker {
                PrimaryRecoveryMarkerState::Absent => {
                    for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
                        harness.app.update();
                    }
                    let window = harness
                        .app
                        .world()
                        .get::<Window>(respawned)
                        .ok_or_else(|| String::from("respawned primary window disappeared"))?;
                    assert_eq!(
                        window.position,
                        WindowPosition::At(RIGHT_DISPLAY_WINDOW_POSITION)
                    );
                },
                PrimaryRecoveryMarkerState::RecoverOnRequest => {
                    for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
                        harness.app.update();
                    }
                    assert_eq!(
                        primary_waiting_work(&harness, &departed.role),
                        WaitingWork::ReapplyRequestOwed
                    );
                    let window = harness
                        .app
                        .world()
                        .get::<Window>(respawned)
                        .ok_or_else(|| String::from("respawned primary window disappeared"))?;
                    assert_eq!(
                        window.position,
                        WindowPosition::At(RIGHT_DISPLAY_WINDOW_POSITION)
                    );
                    let binding_entity = primary_binding_entity(&harness, &departed.role)?;
                    harness.app.world_mut().trigger(ReapplyConfiguration {
                        binding: binding_entity,
                    });
                    wait_for_window_position(&mut harness, respawned, LEFT_DISPLAY_WINDOW_POSITION)
                        .map_err(|error| {
                            format!("request recovery after respawn failed: {error}")
                        })?;
                },
                PrimaryRecoveryMarkerState::RecoverOnReturn => {
                    wait_for_window_position(&mut harness, respawned, LEFT_DISPLAY_WINDOW_POSITION)
                        .map_err(|error| {
                            format!("return recovery after respawn failed: {error}")
                        })?;
                },
            }
        }
        Ok(())
    }

    #[test]
    fn departure_and_window_loss_on_one_update_preserve_opted_in_bindings() -> Result<(), String> {
        for recovery_marker in [
            PrimaryRecoveryMarkerState::RecoverOnRequest,
            PrimaryRecoveryMarkerState::RecoverOnReturn,
        ] {
            let mut harness = ProductionPluginHarness::with_window_plugin_and_window_manager(
                KernelInstallation::WindowManagerOwned,
                WindowPlugin::default(),
                |_| {},
                |path| recovery_marker.configure_window_manager(path),
            )?;
            let (departed_monitor, survivor_monitor) =
                harness.install_identified_headless_display_pair();
            let primary_window = harness.primary_window()?;
            set_primary_window_position(
                &mut harness,
                primary_window,
                LEFT_DISPLAY_WINDOW_POSITION,
            )?;
            harness.app.update();
            let role = persistence::primary_window_role()
                .map_err(|error| format!("failed to create primary role: {error}"))?;
            establish_window_configuration(&mut harness, &role)?;

            harness.disconnect_identified_display(departed_monitor, survivor_monitor)?;
            despawn_primary_window(&mut harness, primary_window)?;
            for _ in 0..RECOVERY_TOPOLOGY_UPDATE_LIMIT {
                harness.app.update();
                if harness
                    .app
                    .world()
                    .resource::<Bindings>()
                    .waiting_work(&role)
                    == recovery_marker.departure_work()
                {
                    break;
                }
            }

            let binding = harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&role)
                .map_err(|error| format!("same-boundary window loss retired the role: {error}"))?;
            assert_eq!(binding.recovery, recovery_marker.recovery_policy());
            assert_eq!(
                harness
                    .app
                    .world()
                    .resource::<Bindings>()
                    .waiting_work(&role),
                recovery_marker.departure_work()
            );
        }
        Ok(())
    }

    #[test]
    fn automatic_return_arriving_while_detached_defers_until_respawn() -> Result<(), String> {
        let (mut harness, departed) =
            departed_primary_window(PrimaryRecoveryMarkerState::RecoverOnReturn)?;
        despawn_primary_window(&mut harness, departed.primary_window)?;
        harness.app.update();
        harness.reconnect_identified_left_display(departed.survivor_monitor);
        for _ in 0..RECOVERY_TOPOLOGY_UPDATE_LIMIT {
            harness.app.update();
        }

        harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&departed.role)
            .map_err(|error| format!("detached primary binding disappeared: {error}"))?;
        assert!(role_is_waiting(&harness.app, &departed.role));
        assert_eq!(
            harness
                .app
                .world()
                .resource::<Bindings>()
                .waiting_work(&departed.role),
            WaitingWork::RestorationOwed
        );
        assert!(window_ledger_holds_nothing(&harness.app, &departed.role));

        let respawned = respawn_primary_window(&mut harness, departed.survivor_monitor);
        wait_for_window_position(&mut harness, respawned, LEFT_DISPLAY_WINDOW_POSITION)?;
        Ok(())
    }

    #[test]
    fn application_request_arriving_while_detached_defers_until_respawn() -> Result<(), String> {
        let (mut harness, departed) =
            departed_primary_window(PrimaryRecoveryMarkerState::RecoverOnRequest)?;
        despawn_primary_window(&mut harness, departed.primary_window)?;
        harness.app.update();
        harness.reconnect_identified_left_display(departed.survivor_monitor);
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
        }
        let binding_entity = primary_binding_entity(&harness, &departed.role)?;
        harness.app.world_mut().trigger(ReapplyConfiguration {
            binding: binding_entity,
        });
        for _ in 0..RECOVERY_TOPOLOGY_UPDATE_LIMIT {
            harness.app.update();
        }

        harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&departed.role)
            .map_err(|error| format!("detached primary binding disappeared: {error}"))?;
        assert!(role_is_waiting(&harness.app, &departed.role));
        assert!(window_ledger_holds_nothing(&harness.app, &departed.role));

        let respawned = respawn_primary_window(&mut harness, departed.survivor_monitor);
        wait_for_window_position(&mut harness, respawned, LEFT_DISPLAY_WINDOW_POSITION)?;
        Ok(())
    }

    #[test]
    fn respawned_stranded_window_starts_fresh_display_and_placement_baselines() -> Result<(), String>
    {
        let (mut harness, departed) =
            departed_primary_window(PrimaryRecoveryMarkerState::RecoverOnReturn)?;
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            if stranded_baselines_are_tracked(&harness, &departed.role) {
                break;
            }
        }
        assert!(stranded_baselines_are_tracked(&harness, &departed.role));

        despawn_primary_window(&mut harness, departed.primary_window)?;
        harness.app.update();
        assert!(!stranded_baselines_are_tracked(&harness, &departed.role));

        respawn_primary_window(&mut harness, departed.survivor_monitor);
        for _ in 0..RECOVERY_TOPOLOGY_UPDATE_LIMIT {
            harness.app.update();
            if stranded_baselines_are_tracked(&harness, &departed.role) {
                break;
            }
        }

        assert!(stranded_baselines_are_tracked(&harness, &departed.role));
        let binding = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&departed.role)
            .map_err(|error| format!("retained primary binding disappeared: {error}"))?;
        assert_eq!(binding.endpoint.device, departed.departed_device);
        Ok(())
    }

    #[test]
    fn recover_on_return_primary_window_goes_home_when_its_display_returns() -> Result<(), String> {
        let (mut harness, departed) =
            departed_primary_window(PrimaryRecoveryMarkerState::RecoverOnReturn)?;
        harness
            .app
            .world_mut()
            .init_resource::<InjectedWinitWindows>();
        harness
            .app
            .world_mut()
            .resource_mut::<InjectedWinitWindows>()
            .insert(departed.primary_window, UVec2::ZERO);
        harness.reconnect_identified_left_display(departed.survivor_monitor);

        let mut returned_home = false;
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            returned_home = harness
                .app
                .world()
                .get::<Window>(departed.primary_window)
                .is_some_and(|window| {
                    window.position == WindowPosition::At(LEFT_DISPLAY_WINDOW_POSITION)
                });
            if returned_home {
                break;
            }
        }

        assert!(returned_home);
        let binding = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&departed.role)
            .map_err(|error| format!("primary window binding disappeared: {error}"))?;
        assert_eq!(binding.endpoint.device, departed.departed_device);
        Ok(())
    }

    #[test]
    fn recover_on_request_primary_window_waits_for_an_explicit_return_request() -> Result<(), String>
    {
        let (mut harness, departed) =
            departed_primary_window(PrimaryRecoveryMarkerState::RecoverOnRequest)?;
        harness
            .app
            .world_mut()
            .init_resource::<InjectedWinitWindows>();
        harness
            .app
            .world_mut()
            .resource_mut::<InjectedWinitWindows>()
            .insert(departed.primary_window, UVec2::ZERO);
        harness.reconnect_identified_left_display(departed.survivor_monitor);
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
        }

        assert_eq!(
            harness
                .app
                .world()
                .resource::<Bindings>()
                .waiting_work(&departed.role),
            WaitingWork::ReapplyRequestOwed
        );
        let window = harness
            .app
            .world()
            .get::<Window>(departed.primary_window)
            .ok_or_else(|| String::from("primary window entity lost its Window"))?;
        assert_eq!(
            window.position,
            WindowPosition::At(RIGHT_DISPLAY_WINDOW_POSITION)
        );
        let binding_entity = primary_binding_entity(&harness, &departed.role)?;
        harness.app.world_mut().trigger(ReapplyConfiguration {
            binding: binding_entity,
        });

        let mut returned_home = false;
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            returned_home = harness
                .app
                .world()
                .get::<Window>(departed.primary_window)
                .is_some_and(|window| {
                    window.position == WindowPosition::At(LEFT_DISPLAY_WINDOW_POSITION)
                });
            if returned_home {
                break;
            }
        }

        assert!(returned_home);
        Ok(())
    }

    #[test]
    fn a_user_move_after_departure_adopts_the_survivor_for_reapply_policies() -> Result<(), String>
    {
        for recovery_marker in [
            PrimaryRecoveryMarkerState::RecoverOnRequest,
            PrimaryRecoveryMarkerState::RecoverOnReturn,
        ] {
            let (mut harness, departed) = departed_primary_window(recovery_marker)?;
            harness.app.update();
            harness.app.update();
            set_primary_window_monitor(
                &mut harness,
                departed.primary_window,
                departed.survivor_monitor,
                RIGHT_DISPLAY_USER_MOVE_POSITION,
            )?;
            wait_for_binding_endpoint(&mut harness, &departed.role, &departed.survivor_device)?;

            let binding = harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&departed.role)
                .map_err(|error| format!("primary window binding disappeared: {error}"))?;
            assert_eq!(binding.recovery, recovery_marker.recovery_policy());
        }
        Ok(())
    }

    #[test]
    fn a_same_offset_move_between_live_displays_rebinds_a_stranded_primary_window()
    -> Result<(), String> {
        let (mut harness, departed) =
            departed_primary_window(PrimaryRecoveryMarkerState::RecoverOnReturn)?;
        harness.app.update();
        harness.app.update();
        let fallback_placement = primary_window_placement(&harness, departed.primary_window)?;

        let rightmost_monitor = harness.add_identified_rightmost_display(departed.survivor_monitor);
        let rightmost_device = device_for_identified_display(RIGHTMOST_IDENTIFIED_DISPLAY_EVIDENCE);
        set_primary_window_monitor(
            &mut harness,
            departed.primary_window,
            rightmost_monitor,
            RIGHTMOST_DISPLAY_WINDOW_POSITION,
        )?;
        wait_for_binding_endpoint(&mut harness, &departed.role, &rightmost_device)?;

        let moved_placement = primary_window_placement(&harness, departed.primary_window)?;
        assert_eq!(moved_placement, fallback_placement);
        let binding = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&departed.role)
            .map_err(|error| format!("primary window binding disappeared: {error}"))?;
        assert_eq!(binding.endpoint.device, rightmost_device);
        assert_eq!(binding.recovery, RecoveryPolicy::ReapplyOnReturn);
        Ok(())
    }

    #[test]
    fn a_startup_revealed_stranded_window_rebinds_when_moved_to_a_live_display()
    -> Result<(), String> {
        let mut harness = ProductionPluginHarness::with_window_plugin_and_window_manager(
            KernelInstallation::WindowManagerOwned,
            WindowPlugin::default(),
            |_| {},
            |path| WindowManagerPlugin::with_path(path).recover_on_return(),
        )?;
        let absent_device = harness.write_absent_classified_primary_placement()?;
        let (_, right_monitor) = harness.install_identified_headless_display_pair();
        let primary_window = harness.primary_window()?;
        set_primary_window_position(&mut harness, primary_window, LEFT_DISPLAY_WINDOW_POSITION)?;
        harness
            .app
            .world_mut()
            .get_mut::<Window>(primary_window)
            .ok_or_else(|| String::from("primary window entity lost its Window"))?
            .visible = false;
        harness
            .app
            .world_mut()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
                EXACT_DISPLAY_WAIT_TIMEOUT_SECS,
            )));
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        for _ in 0..RECOVERY_TOPOLOGY_UPDATE_LIMIT {
            harness.app.update();
            if harness
                .app
                .world()
                .resource::<StrandedWindowMovementBaselines>()
                .is_tracked(&role)
            {
                break;
            }
        }
        harness
            .app
            .world_mut()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));

        let window = harness
            .app
            .world()
            .get::<Window>(primary_window)
            .ok_or_else(|| String::from("primary window entity lost its Window"))?;
        assert!(window.visible);
        assert!(
            harness
                .app
                .world()
                .resource::<recovery::StrandedWindowMovementBaselines>()
                .is_tracked(&role)
        );
        let binding = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&role)
            .map_err(|error| format!("primary window binding disappeared: {error}"))?;
        assert_eq!(binding.endpoint.device, absent_device);

        harness.app.update();
        set_primary_window_monitor(
            &mut harness,
            primary_window,
            right_monitor,
            RIGHT_DISPLAY_USER_MOVE_POSITION,
        )?;
        let right_device = device_for_identified_display(SECOND_IDENTIFIED_DISPLAY_EVIDENCE);
        let related_device_entity = harness
            .app
            .world()
            .get::<LiveDisplayDevices>(right_monitor)
            .ok_or_else(|| String::from("right monitor has no projected display relationship"))?
            .device()
            .map_err(|error| format!("right monitor relationship is not exact: {error}"))?;
        let related_device = harness
            .app
            .world()
            .get::<DeviceKey>(related_device_entity)
            .ok_or_else(|| String::from("related right-display entity has no device key"))?;
        if related_device != &right_device {
            return Err(format!(
                "right monitor relates to {related_device:?}, expected {right_device:?}"
            ));
        }
        wait_for_binding_endpoint(&mut harness, &role, &right_device)?;
        Ok(())
    }

    #[test]
    fn active_only_keeps_the_saved_record_for_a_stranded_primary_window() -> Result<(), String> {
        let mut harness = ProductionPluginHarness::with_window_plugin_and_window_manager(
            KernelInstallation::WindowManagerOwned,
            WindowPlugin::default(),
            |_| {},
            |path| WindowManagerPlugin::with_path(path).recover_on_return(),
        )?;
        let absent_device = harness.write_absent_classified_primary_placement()?;
        harness.install_identified_headless_window_topology();
        harness
            .app
            .insert_resource(ManagedWindowPersistence::ActiveOnly);
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;

        for _ in 0..RECOVERY_TOPOLOGY_UPDATE_LIMIT {
            harness.app.update();
            if harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&role)
                .is_ok_and(|_| role_is_waiting(&harness.app, &role))
            {
                break;
            }
        }

        let binding = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&role)
            .map_err(|error| format!("stranded primary binding was not authored: {error}"))?;
        assert!(role_is_waiting(&harness.app, &role));
        assert_eq!(binding.endpoint.device, absent_device);

        let contents = fs::read_to_string(harness.directory.path().join("windows.ron"))
            .map_err(|error| format!("stranded primary state file is unreadable: {error}"))?;
        let PersistedWindowStateDecodeOutcome::Decoded(decoded) =
            persistence::decode_persisted_state_for_test(&contents)
        else {
            return Err(String::from("stranded primary state file did not decode"));
        };
        assert!(
            decoded.contains_key(&role),
            "ActiveOnly removed the saved record of a managed waiting role"
        );
        Ok(())
    }

    #[test]
    fn a_late_managed_window_is_authored_revealed_and_only_saves_after_a_move() -> Result<(), String>
    {
        let mut harness =
            ProductionPluginHarness::without_primary(KernelInstallation::WindowManagerOwned)?;
        let absent_device = harness.write_absent_classified_managed_placement()?;
        let (_, fallback_monitor) = harness.install_identified_headless_display_pair();
        harness.app.update();
        harness
            .app
            .insert_resource(LateManagedWindowRegistration::Pending {
                monitor: fallback_monitor,
            })
            .add_systems(
                Update,
                (register_late_managed_window, ApplyDeferred)
                    .chain()
                    .before(ClerestoryWindowDriverSet::BindingAuthoring),
            );

        harness.app.update();

        let managed_window = (*harness
            .app
            .world()
            .resource::<LateManagedWindowRegistration>())
        .window()?;
        let role = persistence::managed_window_role("late-inspector")
            .map_err(|error| format!("failed to create managed role: {error}"))?;
        let window = harness
            .app
            .world()
            .get::<Window>(managed_window)
            .ok_or_else(|| String::from("late managed entity lost its Window"))?;
        assert!(window.visible);
        assert!(
            harness
                .app
                .world()
                .get::<visibility::SavedDisplayRevealWait>(managed_window)
                .is_none()
        );
        assert!(
            harness
                .app
                .world()
                .get::<managed::WindowBindingAuthoring>(managed_window)
                .is_some()
        );
        let binding = harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&role)
            .map_err(|error| format!("late managed binding was not authored: {error}"))?;
        assert_eq!(binding.endpoint.device, absent_device);
        assert_eq!(binding.recovery, RecoveryPolicy::Forget);

        let contents = fs::read_to_string(harness.directory.path().join("windows.ron"))
            .map_err(|error| format!("unmoved managed state file is unreadable: {error}"))?;
        let PersistedWindowStateDecodeOutcome::Decoded(decoded) =
            persistence::decode_persisted_state_for_test(&contents)
        else {
            return Err(String::from("unmoved managed state file did not decode"));
        };
        let persisted = decoded
            .get(&role)
            .ok_or_else(|| String::from("unmoved managed state file lost its role"))?;
        assert_eq!(
            persisted.target,
            PersistedWindowTargetV5::Classified(absent_device)
        );

        harness.app.update();
        harness.app.update();
        harness
            .app
            .world_mut()
            .get_mut::<Window>(managed_window)
            .ok_or_else(|| String::from("late managed entity lost its Window before moving"))?
            .position = WindowPosition::At(RIGHT_DISPLAY_USER_MOVE_POSITION);
        harness.app.update();
        let fallback_device = device_for_identified_display(SECOND_IDENTIFIED_DISPLAY_EVIDENCE);
        wait_for_binding_endpoint(&mut harness, &role, &fallback_device)?;
        establish_window_configuration(&mut harness, &role)?;

        let contents = fs::read_to_string(harness.directory.path().join("windows.ron"))
            .map_err(|error| format!("moved managed state file is unreadable: {error}"))?;
        let PersistedWindowStateDecodeOutcome::Decoded(decoded) =
            persistence::decode_persisted_state_for_test(&contents)
        else {
            return Err(String::from("moved managed state file did not decode"));
        };
        let persisted = decoded
            .get(&role)
            .ok_or_else(|| String::from("moved managed state file lost its role"))?;
        assert_eq!(
            persisted.target,
            PersistedWindowTargetV5::Classified(fallback_device)
        );
        assert_eq!(
            persisted.position,
            persistence::PersistedPosition::MonitorOffset(
                RIGHT_DISPLAY_USER_MOVE_POSITION - IVec2::new(1_920, 0)
            )
        );
        Ok(())
    }

    #[test]
    fn a_runtime_created_managed_window_reaches_ready_and_is_persisted() -> Result<(), String> {
        for retention in [
            ManagedWindowPersistence::RememberAll,
            ManagedWindowPersistence::ActiveOnly,
        ] {
            let mut harness =
                ProductionPluginHarness::without_primary(KernelInstallation::WindowManagerOwned)?;
            harness
                .app
                .insert_resource(retention.clone())
                .init_resource::<InjectedWinitWindows>()
                .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
                    SETTLE_STABILITY_SECS,
                )));
            let monitor = harness.install_identified_headless_window_topology();
            harness.app.update();

            let mut window = Window {
                position: WindowPosition::At(LEFT_DISPLAY_WINDOW_POSITION),
                ..default()
            };
            window.resolution.set(640.0, 480.0);
            let managed_window = harness
                .app
                .world_mut()
                .spawn((
                    window,
                    ManagedWindowName("window-1".into()),
                    OnMonitor(monitor),
                ))
                .id();
            harness
                .app
                .world_mut()
                .resource_mut::<InjectedWinitWindows>()
                .insert(managed_window, UVec2::ZERO);
            let role = persistence::managed_window_role("window-1")
                .map_err(|error| format!("failed to create managed role: {error}"))?;

            for _ in 0..RECOVERY_TOPOLOGY_UPDATE_LIMIT {
                harness.app.update();
                if harness
                    .app
                    .world()
                    .resource::<Bindings>()
                    .binding(&role)
                    .is_ok_and(|binding| {
                        role_is_established(&harness.app, &role)
                            && binding.last_known_good().is_ok()
                    })
                {
                    break;
                }
            }

            let binding = harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&role)
                .map_err(|error| format!("runtime-created managed binding disappeared: {error}"))?;
            assert!(role_is_established(&harness.app, &role));
            assert!(binding.last_known_good().is_ok());

            // Establishment occurs after persistence in the update schedule. Give persistence one
            // settled frame in which to observe the newly writable binding.
            harness.app.update();

            let contents = fs::read_to_string(harness.directory.path().join("windows.ron"))
                .map_err(|error| format!("managed window state file is unreadable: {error}"))?;
            let PersistedWindowStateDecodeOutcome::Decoded(decoded) =
                persistence::decode_persisted_state_for_test(&contents)
            else {
                return Err(String::from("managed window state file did not decode"));
            };
            assert!(
                decoded.contains_key(&role),
                "the ready runtime-created managed role was not written under {retention:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn reapply_primary_windows_rebind_immediately_between_live_displays() -> Result<(), String> {
        for recovery_marker in [
            PrimaryRecoveryMarkerState::RecoverOnRequest,
            PrimaryRecoveryMarkerState::RecoverOnReturn,
        ] {
            let mut harness = ProductionPluginHarness::with_window_plugin_and_window_manager(
                KernelInstallation::WindowManagerOwned,
                WindowPlugin::default(),
                |_| {},
                |path| recovery_marker.configure_window_manager(path),
            )?;
            let (_, right) = harness.install_identified_headless_display_pair();
            let primary_window = harness.primary_window()?;
            set_primary_window_position(
                &mut harness,
                primary_window,
                LEFT_DISPLAY_WINDOW_POSITION,
            )?;
            harness.app.update();
            let role = persistence::primary_window_role()
                .map_err(|error| format!("failed to create primary role: {error}"))?;
            establish_window_configuration(&mut harness, &role)?;
            let right_device = device_for_identified_display(SECOND_IDENTIFIED_DISPLAY_EVIDENCE);
            set_primary_window_monitor(
                &mut harness,
                primary_window,
                right,
                RIGHT_DISPLAY_WINDOW_POSITION,
            )?;
            wait_for_binding_endpoint(&mut harness, &role, &right_device)?;

            let binding = harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&role)
                .map_err(|error| format!("primary window binding disappeared: {error}"))?;
            assert_eq!(binding.recovery, recovery_marker.recovery_policy());
        }
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

        let mut attempt = None;
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            if let Some(started) = applying_attempt(&harness.app, &role) {
                attempt = Some(started);
                break;
            }
        }
        // The move below happens while this apply is still in flight, which is the only ordering a
        // real drag produces: moving a window is itself what puts the role into `Applying`, so a
        // rebind that waits for an established role never runs. Settling the apply first hid
        // exactly that defect.
        attempt.ok_or_else(|| String::from("primary window never started an apply"))?;

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
            monitors::DisplayFingerprint::from_evidence_bytes(SECOND_IDENTIFIED_DISPLAY_EVIDENCE);
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

        let mut attempt = None;
        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            if let Some(started) = applying_attempt(&harness.app, &role) {
                attempt = Some(started);
                break;
            }
        }
        let attempt =
            attempt.ok_or_else(|| String::from("primary window never started an apply"))?;
        finish_window_as_dispatched(&mut harness, attempt)?;
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
    fn simultaneous_window_attempt_results_are_isolated_by_attempt() -> Result<(), String> {
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
            ManagedWindowName("attempt-isolation".into()),
            OnMonitor(monitor_entity),
        ));
        let primary_role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let managed_role = persistence::managed_window_role("attempt-isolation")
            .map_err(|error| format!("failed to create managed role: {error}"))?;

        for _ in 0..STARTUP_RESTORE_UPDATE_LIMIT {
            harness.app.update();
            if applying_attempt(&harness.app, &primary_role).is_some()
                && applying_attempt(&harness.app, &managed_role).is_some()
            {
                break;
            }
        }
        let primary_attempt = applying_attempt(&harness.app, &primary_role)
            .ok_or_else(|| String::from("primary window did not start its attempt"))?;
        let managed_attempt = applying_attempt(&harness.app, &managed_role)
            .ok_or_else(|| String::from("managed window did not start its attempt"))?;
        assert_ne!(primary_attempt, managed_attempt);

        finish_window_as_dispatched(&mut harness, primary_attempt)?;
        harness.app.update();
        harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&primary_role)
            .map_err(|error| format!("primary binding disappeared: {error}"))?;
        assert!(role_is_established(&harness.app, &primary_role));
        assert_eq!(
            applying_attempt(&harness.app, &managed_role),
            Some(managed_attempt),
        );
        Ok(())
    }
}
