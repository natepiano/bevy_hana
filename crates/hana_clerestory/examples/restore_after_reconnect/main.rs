//! Kernel-backed monitor reconnect probe.

mod constants;
mod control;
mod remote;
mod setup;
mod trace;
mod trace_store;

use std::env::VarError;
use std::env::var;
use std::fs::remove_file;
use std::io::Error;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::id;

use bevy::prelude::App;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Last;
use bevy::prelude::Plugin;
use bevy::prelude::PluginGroup;
use bevy::prelude::Resource;
use bevy::prelude::Startup;
use bevy::prelude::Update;
use bevy::prelude::WindowPlugin;
use bevy::prelude::default;
use bevy::window::ExitCondition;
use bevy::window::MonitorSelection;
use bevy::window::VideoModeSelection;
use bevy::window::WindowMode;
use constants::DEFAULT_EXTERNAL_MONITOR_INDEX;
use constants::DEFAULT_PROBE_PORT;
use constants::EXIT_AFTER_FRAME_ENVIRONMENT_VARIABLE;
use constants::MONITOR_INDEX_ENVIRONMENT_VARIABLE;
use constants::PERSISTENCE_FILE_PREFIX;
use constants::PROBE_BOOT_NONCE_ENVIRONMENT_VARIABLE;
use constants::PROBE_CAPABILITY_ENVIRONMENT_VARIABLE;
use constants::PROBE_PERSISTENCE_PATH_ENVIRONMENT_VARIABLE;
use constants::PROBE_PORT_ENVIRONMENT_VARIABLE;
use constants::PROBE_RUN_ID_ENVIRONMENT_VARIABLE;
use constants::STARTUP_MODE_BORDERLESS;
use constants::STARTUP_MODE_ENVIRONMENT_VARIABLE;
use constants::STARTUP_MODE_EXCLUSIVE;
use constants::STARTUP_MODE_WINDOWED;
use hana_clerestory::WindowManagerPlugin;

struct HotplugProbePlugin;

impl Plugin for HotplugProbePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<control::ApplicationRecoveryLifecycle>()
            .init_resource::<control::CommandReceipts>()
            .init_resource::<remote::ProbeReadiness>()
            .init_resource::<trace::ProbeTrace>()
            .add_observer(control::apply_probe_command)
            .add_observer(control::on_application_role_status_changed)
            .add_observer(trace::on_device_arrived)
            .add_observer(trace::on_device_departed)
            .add_observer(trace::on_identity_question_raised)
            .add_observer(trace::on_probe_window_added)
            .add_observer(trace::on_live_role_changed)
            .add_observer(trace::on_window_restore_mismatch)
            .add_observer(trace::on_window_restored)
            .add_systems(
                Startup,
                (setup::spawn_probe_windows, setup::trace_probe_session).chain(),
            )
            .add_systems(
                Update,
                (
                    setup::register_probe_windows_on_selected_monitor,
                    setup::control_automatic_window_mode,
                    setup::retire_automatic_window,
                    control::despawn_requested_windows,
                )
                    .chain(),
            )
            .add_systems(
                Last,
                (
                    remote::record_probe_readiness,
                    setup::exit_after_smoke_frame,
                )
                    .chain(),
            );
    }
}

/// Deterministic initial `WindowMode` for the managed automatic window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Resource)]
enum ProbeStartupMode {
    #[default]
    Windowed,
    Borderless,
    Exclusive,
}

impl ProbeStartupMode {
    /// Maps to the winit [`WindowMode`] the automatic window is created with. `monitor_index`
    /// reaches only that initial placement; recovery goes to the one exact reported display the
    /// clerestory binding later authorizes, not to this index.
    const fn automatic_window_mode(self, monitor_index: usize) -> WindowMode {
        match self {
            Self::Windowed => WindowMode::Windowed,
            Self::Borderless => {
                WindowMode::BorderlessFullscreen(MonitorSelection::Index(monitor_index))
            },
            Self::Exclusive => WindowMode::Fullscreen(
                MonitorSelection::Index(monitor_index),
                VideoModeSelection::Current,
            ),
        }
    }

    /// Documented `CLERESTORY_PROBE_STARTUP_MODE` spelling for status records.
    const fn selector(self) -> &'static str {
        match self {
            Self::Windowed => STARTUP_MODE_WINDOWED,
            Self::Borderless => STARTUP_MODE_BORDERLESS,
            Self::Exclusive => STARTUP_MODE_EXCLUSIVE,
        }
    }
}

#[derive(Clone, Copy, Resource)]
struct SmokeExitFrame(u32);

/// Requested monitor before startup, or the live fallback selected when it is unavailable.
///
/// The existing `selected_monitor_index` wire field serializes the requested index for
/// `Requested` and the active index for `Fallback`. A fallback logs
/// `requested monitor index {requested} is unavailable; using monitor index {active}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Resource)]
enum ProbeMonitorSelection {
    Requested(usize),
    Fallback { requested: usize, active: usize },
}

impl ProbeMonitorSelection {
    const fn selected_monitor_index(self) -> usize {
        match self {
            Self::Requested(requested) => requested,
            Self::Fallback { active, .. } => active,
        }
    }

    const fn requested_monitor_index(self) -> usize {
        match self {
            Self::Requested(requested) | Self::Fallback { requested, .. } => requested,
        }
    }

    const fn use_fallback(&mut self, active: usize) {
        *self = Self::Fallback {
            requested: self.requested_monitor_index(),
            active,
        };
    }
}

enum ProbePersistencePath {
    Supplied(PathBuf),
    Derived(PathBuf),
}

#[derive(Clone, Copy)]
enum ProbeExitBehavior {
    Continue,
    ExitAfter(SmokeExitFrame),
}

fn invalid_unicode_environment_value(name: &str) -> Error {
    Error::new(
        ErrorKind::InvalidInput,
        format!("{name} must contain Unicode text"),
    )
}

fn parse_startup_mode(value: &str) -> std::io::Result<ProbeStartupMode> {
    match value {
        STARTUP_MODE_WINDOWED => Ok(ProbeStartupMode::Windowed),
        STARTUP_MODE_BORDERLESS => Ok(ProbeStartupMode::Borderless),
        STARTUP_MODE_EXCLUSIVE => Ok(ProbeStartupMode::Exclusive),
        other => Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "invalid {STARTUP_MODE_ENVIRONMENT_VARIABLE}: {other:?} (expected \
                 {STARTUP_MODE_WINDOWED}, {STARTUP_MODE_BORDERLESS}, or {STARTUP_MODE_EXCLUSIVE})"
            ),
        )),
    }
}

fn selected_startup_mode() -> std::io::Result<ProbeStartupMode> {
    match var(STARTUP_MODE_ENVIRONMENT_VARIABLE) {
        Ok(value) => parse_startup_mode(&value),
        Err(VarError::NotPresent) => Ok(ProbeStartupMode::default()),
        Err(VarError::NotUnicode(_)) => Err(invalid_unicode_environment_value(
            STARTUP_MODE_ENVIRONMENT_VARIABLE,
        )),
    }
}

fn persistence_path() -> std::io::Result<ProbePersistencePath> {
    match var(PROBE_PERSISTENCE_PATH_ENVIRONMENT_VARIABLE) {
        Ok(path) => Ok(ProbePersistencePath::Supplied(PathBuf::from(path))),
        Err(VarError::NotPresent) => Ok(ProbePersistencePath::Derived(
            std::env::temp_dir().join(format!("{PERSISTENCE_FILE_PREFIX}-{}.ron", id())),
        )),
        Err(VarError::NotUnicode(_)) => Err(invalid_unicode_environment_value(
            PROBE_PERSISTENCE_PATH_ENVIRONMENT_VARIABLE,
        )),
    }
}

fn probe_port() -> std::io::Result<u16> {
    match var(PROBE_PORT_ENVIRONMENT_VARIABLE) {
        Ok(value) => value.parse().map_err(|error| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("invalid {PROBE_PORT_ENVIRONMENT_VARIABLE}: {error}"),
            )
        }),
        Err(VarError::NotPresent) => Ok(DEFAULT_PROBE_PORT),
        Err(VarError::NotUnicode(_)) => Err(invalid_unicode_environment_value(
            PROBE_PORT_ENVIRONMENT_VARIABLE,
        )),
    }
}

fn probe_monitor_selection() -> std::io::Result<ProbeMonitorSelection> {
    match var(MONITOR_INDEX_ENVIRONMENT_VARIABLE) {
        Ok(value) => value
            .parse()
            .map(ProbeMonitorSelection::Requested)
            .map_err(|error| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("invalid {MONITOR_INDEX_ENVIRONMENT_VARIABLE}: {error}"),
                )
            }),
        Err(VarError::NotPresent) => Ok(ProbeMonitorSelection::Requested(
            DEFAULT_EXTERNAL_MONITOR_INDEX,
        )),
        Err(VarError::NotUnicode(_)) => Err(invalid_unicode_environment_value(
            MONITOR_INDEX_ENVIRONMENT_VARIABLE,
        )),
    }
}

fn probe_run_id(process_id: u32) -> std::io::Result<String> {
    match var(PROBE_RUN_ID_ENVIRONMENT_VARIABLE) {
        Ok(run_id) => Ok(run_id),
        Err(VarError::NotPresent) => Ok(format!("manual-{process_id}")),
        Err(VarError::NotUnicode(_)) => Err(invalid_unicode_environment_value(
            PROBE_RUN_ID_ENVIRONMENT_VARIABLE,
        )),
    }
}

fn probe_boot_nonce(process_id: u32) -> std::io::Result<String> {
    match var(PROBE_BOOT_NONCE_ENVIRONMENT_VARIABLE) {
        Ok(boot_nonce) => Ok(boot_nonce),
        Err(VarError::NotPresent) => Ok(format!("boot-{process_id}")),
        Err(VarError::NotUnicode(_)) => Err(invalid_unicode_environment_value(
            PROBE_BOOT_NONCE_ENVIRONMENT_VARIABLE,
        )),
    }
}

fn probe_capability(process_id: u32) -> std::io::Result<String> {
    match var(PROBE_CAPABILITY_ENVIRONMENT_VARIABLE) {
        Ok(capability) => Ok(capability),
        Err(VarError::NotPresent) => Ok(format!("local-{process_id}")),
        Err(VarError::NotUnicode(_)) => Err(invalid_unicode_environment_value(
            PROBE_CAPABILITY_ENVIRONMENT_VARIABLE,
        )),
    }
}

fn probe_session() -> std::io::Result<remote::ProbeSession> {
    let process_id = id();
    let run_id = probe_run_id(process_id)?;
    let boot_nonce = probe_boot_nonce(process_id)?;
    let capability = probe_capability(process_id)?;
    Ok(remote::ProbeSession::new(run_id, boot_nonce, capability))
}

fn fresh_persistence_path() -> std::io::Result<PathBuf> {
    prepare_persistence_path(persistence_path()?)
}

fn prepare_persistence_path(selection: ProbePersistencePath) -> std::io::Result<PathBuf> {
    let path = match selection {
        ProbePersistencePath::Supplied(path) => return Ok(path),
        ProbePersistencePath::Derived(path) => path,
    };
    match remove_file(&path) {
        Ok(()) => {},
        Err(error) if error.kind() == ErrorKind::NotFound => {},
        Err(error) => return Err(error),
    }
    Ok(path)
}

fn smoke_exit_frame() -> std::io::Result<ProbeExitBehavior> {
    match var(EXIT_AFTER_FRAME_ENVIRONMENT_VARIABLE) {
        Ok(value) => value
            .parse()
            .map(|frame| ProbeExitBehavior::ExitAfter(SmokeExitFrame(frame)))
            .map_err(|error| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("invalid {EXIT_AFTER_FRAME_ENVIRONMENT_VARIABLE}: {error}"),
                )
            }),
        Err(VarError::NotPresent) => Ok(ProbeExitBehavior::Continue),
        Err(VarError::NotUnicode(_)) => Err(invalid_unicode_environment_value(
            EXIT_AFTER_FRAME_ENVIRONMENT_VARIABLE,
        )),
    }
}

fn main() -> std::io::Result<()> {
    let startup_mode = selected_startup_mode()?;
    let smoke_exit_frame = smoke_exit_frame()?;
    let persistence_path = fresh_persistence_path()?;
    let probe_monitor_selection = probe_monitor_selection()?;
    let probe_port = probe_port()?;
    let probe_session = probe_session()?;
    let mut app = App::new();
    app.insert_resource(startup_mode)
        .insert_resource(probe_monitor_selection)
        .insert_resource(probe_session)
        .add_plugins(HotplugProbePlugin)
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: None,
            exit_condition: ExitCondition::DontExit,
            ..default()
        }))
        .add_plugins(remote::plugin())
        .add_plugins(remote::http_plugin(probe_port))
        .add_plugins(WindowManagerPlugin::with_path(persistence_path).recover_on_return());
    if let ProbeExitBehavior::ExitAfter(smoke_exit_frame) = smoke_exit_frame {
        app.insert_resource(smoke_exit_frame);
    }
    app.run();
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::error::Error as StdError;

    use super::*;

    #[test]
    fn startup_mode_selector_parses_each_documented_value_and_defaults_to_windowed() {
        assert_eq!(ProbeStartupMode::default(), ProbeStartupMode::Windowed);
        assert_eq!(
            parse_startup_mode(STARTUP_MODE_WINDOWED).expect("windowed should parse"),
            ProbeStartupMode::Windowed,
        );
        assert_eq!(
            parse_startup_mode(STARTUP_MODE_BORDERLESS).expect("borderless should parse"),
            ProbeStartupMode::Borderless,
        );
        assert_eq!(
            parse_startup_mode(STARTUP_MODE_EXCLUSIVE).expect("exclusive should parse"),
            ProbeStartupMode::Exclusive,
        );
    }

    #[test]
    fn startup_mode_selector_rejects_unknown_values_naming_the_variable() {
        let error = parse_startup_mode("fullscreen")
            .expect_err("undocumented selector value should be rejected");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(
            error
                .to_string()
                .contains(STARTUP_MODE_ENVIRONMENT_VARIABLE)
        );
    }

    #[test]
    fn startup_mode_trace_spelling_matches_the_documented_selector_values() {
        assert_eq!(ProbeStartupMode::Windowed.selector(), STARTUP_MODE_WINDOWED);
        assert_eq!(
            ProbeStartupMode::Borderless.selector(),
            STARTUP_MODE_BORDERLESS
        );
        assert_eq!(
            ProbeStartupMode::Exclusive.selector(),
            STARTUP_MODE_EXCLUSIVE
        );
    }

    #[test]
    fn fullscreen_startup_modes_target_the_controller_selected_monitor() {
        let selected_monitor = 3;
        assert_eq!(
            ProbeStartupMode::Borderless.automatic_window_mode(selected_monitor),
            WindowMode::BorderlessFullscreen(MonitorSelection::Index(selected_monitor)),
        );
        assert_eq!(
            ProbeStartupMode::Exclusive.automatic_window_mode(selected_monitor),
            WindowMode::Fullscreen(
                MonitorSelection::Index(selected_monitor),
                VideoModeSelection::Current,
            ),
        );
    }

    #[test]
    fn explicitly_supplied_persistence_path_is_preserved() -> Result<(), Box<dyn StdError>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("windows.ron");
        std::fs::write(&path, "saved state")?;

        let selected_path = prepare_persistence_path(ProbePersistencePath::Supplied(path.clone()))?;

        assert_eq!(selected_path, path);
        assert_eq!(std::fs::read_to_string(selected_path)?, "saved state");
        Ok(())
    }
}
