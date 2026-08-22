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
            .add_observer(control::on_application_role_available)
            .add_observer(control::on_application_role_awaiting)
            .add_observer(trace::on_attempt_finished)
            .add_observer(trace::on_device_arrived)
            .add_observer(trace::on_device_departed)
            .add_observer(trace::on_identity_question_raised)
            .add_observer(trace::on_probe_window_added)
            .add_observer(trace::on_recovery_policy_changed)
            .add_observer(trace::on_role_available)
            .add_observer(trace::on_role_awaiting)
            .add_observer(trace::on_role_state_changed)
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Resource)]
enum ProbeStartupMode {
    Windowed,
    Borderless,
    Exclusive,
}

impl ProbeStartupMode {
    /// Chooses only a winit startup mode. The clerestory binding later authorizes one exact
    /// reported display; this startup hint does not select a display for recovery.
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

#[derive(Resource)]
struct SmokeExitFrame(u32);

#[derive(Resource)]
struct ProbeMonitorIndex(usize);

fn optional_environment_value(name: &str) -> std::io::Result<Option<String>> {
    match var(name) {
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(Error::new(
            ErrorKind::InvalidInput,
            format!("{name} must contain Unicode text"),
        )),
    }
}

fn parse_startup_mode(value: Option<&str>) -> std::io::Result<ProbeStartupMode> {
    match value {
        None | Some(STARTUP_MODE_WINDOWED) => Ok(ProbeStartupMode::Windowed),
        Some(STARTUP_MODE_BORDERLESS) => Ok(ProbeStartupMode::Borderless),
        Some(STARTUP_MODE_EXCLUSIVE) => Ok(ProbeStartupMode::Exclusive),
        Some(other) => Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "invalid {STARTUP_MODE_ENVIRONMENT_VARIABLE}: {other:?} (expected \
                 {STARTUP_MODE_WINDOWED}, {STARTUP_MODE_BORDERLESS}, or {STARTUP_MODE_EXCLUSIVE})"
            ),
        )),
    }
}

fn selected_startup_mode() -> std::io::Result<ProbeStartupMode> {
    parse_startup_mode(optional_environment_value(STARTUP_MODE_ENVIRONMENT_VARIABLE)?.as_deref())
}

fn persistence_path() -> std::io::Result<PathBuf> {
    Ok(
        optional_environment_value(PROBE_PERSISTENCE_PATH_ENVIRONMENT_VARIABLE)?.map_or_else(
            || std::env::temp_dir().join(format!("{PERSISTENCE_FILE_PREFIX}-{}.ron", id())),
            PathBuf::from,
        ),
    )
}

fn probe_port() -> std::io::Result<u16> {
    optional_environment_value(PROBE_PORT_ENVIRONMENT_VARIABLE)?.map_or(
        Ok(DEFAULT_PROBE_PORT),
        |value| {
            value.parse().map_err(|error| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("invalid {PROBE_PORT_ENVIRONMENT_VARIABLE}: {error}"),
                )
            })
        },
    )
}

fn probe_monitor_index() -> std::io::Result<ProbeMonitorIndex> {
    optional_environment_value(MONITOR_INDEX_ENVIRONMENT_VARIABLE)?.map_or(
        Ok(ProbeMonitorIndex(DEFAULT_EXTERNAL_MONITOR_INDEX)),
        |value| {
            value.parse().map(ProbeMonitorIndex).map_err(|error| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("invalid {MONITOR_INDEX_ENVIRONMENT_VARIABLE}: {error}"),
                )
            })
        },
    )
}

fn probe_session() -> std::io::Result<remote::ProbeSession> {
    let process_id = id();
    let run_id = optional_environment_value(PROBE_RUN_ID_ENVIRONMENT_VARIABLE)?
        .unwrap_or_else(|| format!("manual-{process_id}"));
    let boot_nonce = optional_environment_value(PROBE_BOOT_NONCE_ENVIRONMENT_VARIABLE)?
        .unwrap_or_else(|| format!("boot-{process_id}"));
    let capability = optional_environment_value(PROBE_CAPABILITY_ENVIRONMENT_VARIABLE)?
        .unwrap_or_else(|| format!("local-{process_id}"));
    Ok(remote::ProbeSession::new(run_id, boot_nonce, capability))
}

fn fresh_persistence_path() -> std::io::Result<PathBuf> {
    let path = persistence_path()?;
    match remove_file(&path) {
        Ok(()) => {},
        Err(error) if error.kind() == ErrorKind::NotFound => {},
        Err(error) => return Err(error),
    }
    Ok(path)
}

fn smoke_exit_frame() -> std::io::Result<Option<u32>> {
    optional_environment_value(EXIT_AFTER_FRAME_ENVIRONMENT_VARIABLE)?
        .map(|value| {
            value.parse().map_err(|error| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("invalid {EXIT_AFTER_FRAME_ENVIRONMENT_VARIABLE}: {error}"),
                )
            })
        })
        .transpose()
}

fn main() -> std::io::Result<()> {
    let startup_mode = selected_startup_mode()?;
    let smoke_exit_frame = smoke_exit_frame()?;
    let persistence_path = fresh_persistence_path()?;
    let probe_monitor_index = probe_monitor_index()?;
    let probe_port = probe_port()?;
    let probe_session = probe_session()?;
    let mut app = App::new();
    app.insert_resource(startup_mode)
        .insert_resource(probe_monitor_index)
        .insert_resource(probe_session)
        .add_plugins(HotplugProbePlugin)
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: None,
            exit_condition: ExitCondition::DontExit,
            ..default()
        }))
        .add_plugins(remote::plugin())
        .add_plugins(remote::http_plugin(probe_port))
        .add_plugins(WindowManagerPlugin::with_path(persistence_path));
    if let Some(frame) = smoke_exit_frame {
        app.insert_resource(SmokeExitFrame(frame));
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
    use super::*;

    #[test]
    fn startup_mode_selector_parses_each_documented_value_and_defaults_to_windowed() {
        assert_eq!(
            parse_startup_mode(None).expect("absent selector should default"),
            ProbeStartupMode::Windowed,
        );
        assert_eq!(
            parse_startup_mode(Some(STARTUP_MODE_WINDOWED)).expect("windowed should parse"),
            ProbeStartupMode::Windowed,
        );
        assert_eq!(
            parse_startup_mode(Some(STARTUP_MODE_BORDERLESS)).expect("borderless should parse"),
            ProbeStartupMode::Borderless,
        );
        assert_eq!(
            parse_startup_mode(Some(STARTUP_MODE_EXCLUSIVE)).expect("exclusive should parse"),
            ProbeStartupMode::Exclusive,
        );
    }

    #[test]
    fn startup_mode_selector_rejects_unknown_values_naming_the_variable() {
        let error = parse_startup_mode(Some("fullscreen"))
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
}
