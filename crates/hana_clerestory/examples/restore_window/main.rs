//! Interactive example for testing window restoration, fullscreen modes, and multi-window
//! management.
//!
//! Run with: `cargo run --example restore_window`
//!
//! Controls (all windows):
//! - Press `Enter` for exclusive fullscreen (uses selected video mode)
//! - Press `B` for borderless fullscreen
//! - Press `W` for windowed mode
//! - Press `Up`/`Down` to cycle through available video modes
//! - Press `Space` to spawn a new managed window
//! - Press `P` to toggle persistence mode (`RememberAll` / `ActiveOnly`)
//! - Press `Ctrl+Shift+Backspace` to clear saved state and quit
//! - Press `Q` to quit

mod constants;
mod debug;
mod display;
mod events;
mod input;
mod mode_observers;
mod remote;
mod setup;

use std::env::VarError;
use std::env::var;
use std::io::Error;
use std::io::ErrorKind;

use bevy::pbr::PbrPlugin;
use bevy::prelude::App;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::IVec2;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::PluginGroup;
use bevy::prelude::Startup;
use bevy::prelude::Update;
use bevy::prelude::Window;
use bevy::prelude::WindowPlugin;
use bevy::prelude::default;
use bevy::window::MonitorSelection;
use bevy::window::WindowPosition;
use bevy::window::WindowResolution;
use constants::PRIMARY_WINDOW_TITLE;
use constants::TEST_LAUNCH_MONITOR_ENVIRONMENT_VARIABLE;
use constants::TEST_LAUNCH_POSITION_ENVIRONMENT_VARIABLE;
use constants::TEST_LAUNCH_SIZE_ENVIRONMENT_VARIABLE;
use constants::TEST_MODE_ENVIRONMENT_VARIABLE;
use constants::TEST_PERSISTENCE_PATH_ENVIRONMENT_VARIABLE;
use events::MismatchStates;
use events::RestoredStates;
use events::WindowsSettledCount;
use hana_clerestory::WindowManagerPlugin;
use input::KeyboardInputMode;
use input::SelectedVideoModes;
use setup::WindowCounter;

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

fn parse_launch_position(
    monitor: Option<&str>,
    position: Option<&str>,
) -> std::io::Result<WindowPosition> {
    if let Some(position) = position {
        let (x, y) = position.split_once(',').ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("invalid {TEST_LAUNCH_POSITION_ENVIRONMENT_VARIABLE}: expected x,y"),
            )
        })?;
        let x = x.parse::<i32>().map_err(|error| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("invalid {TEST_LAUNCH_POSITION_ENVIRONMENT_VARIABLE} x: {error}"),
            )
        })?;
        let y = y.parse::<i32>().map_err(|error| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("invalid {TEST_LAUNCH_POSITION_ENVIRONMENT_VARIABLE} y: {error}"),
            )
        })?;
        return Ok(WindowPosition::At(IVec2::new(x, y)));
    }
    monitor.map_or(Ok(WindowPosition::Automatic), |value| {
        let monitor_index = value.parse::<usize>().map_err(|error| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("invalid {TEST_LAUNCH_MONITOR_ENVIRONMENT_VARIABLE}: {error}"),
            )
        })?;
        Ok(WindowPosition::Centered(MonitorSelection::Index(
            monitor_index,
        )))
    })
}

fn test_launch_position() -> std::io::Result<WindowPosition> {
    parse_launch_position(
        optional_environment_value(TEST_LAUNCH_MONITOR_ENVIRONMENT_VARIABLE)?.as_deref(),
        optional_environment_value(TEST_LAUNCH_POSITION_ENVIRONMENT_VARIABLE)?.as_deref(),
    )
}

/// Parse `CLERESTORY_TEST_LAUNCH_SIZE` (physical `width,height`) into a window resolution.
/// Returns `None` when unset, keeping Bevy's default window size for non-cross-DPI cases.
fn parse_launch_size(value: Option<&str>) -> std::io::Result<Option<WindowResolution>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let (width, height) = value.split_once(',').ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("invalid {TEST_LAUNCH_SIZE_ENVIRONMENT_VARIABLE}: expected width,height"),
        )
    })?;
    let width = width.trim().parse::<u32>().map_err(|error| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("invalid {TEST_LAUNCH_SIZE_ENVIRONMENT_VARIABLE} width: {error}"),
        )
    })?;
    let height = height.trim().parse::<u32>().map_err(|error| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("invalid {TEST_LAUNCH_SIZE_ENVIRONMENT_VARIABLE} height: {error}"),
        )
    })?;
    Ok(Some(WindowResolution::new(width, height)))
}

fn test_launch_size() -> std::io::Result<Option<WindowResolution>> {
    parse_launch_size(optional_environment_value(TEST_LAUNCH_SIZE_ENVIRONMENT_VARIABLE)?.as_deref())
}

fn main() -> std::io::Result<()> {
    let launch_position = test_launch_position()?;
    let launch_size = test_launch_size()?;
    let persistence_path = optional_environment_value(TEST_PERSISTENCE_PATH_ENVIRONMENT_VARIABLE)?;
    let mut primary_window = Window {
        title: PRIMARY_WINDOW_TITLE.into(),
        position: launch_position,
        ..default()
    };
    if let Some(resolution) = launch_size {
        primary_window.resolution = resolution;
    }
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(primary_window),
                ..default()
            })
            // This window manager renders only flat UI, so GPU mesh preprocessing and its
            // frustum-culling compute pass are pure overhead. Disabling them also avoids a
            // startup crash on GPUs whose `max_storage_buffers_per_shader_stage` is below the
            // 8 that the frustum-culling bind group requires (e.g. Asahi/Mesa, limit 6).
            .set(PbrPlugin {
                use_gpu_instance_buffer_builder: false,
                ..default()
            }),
    );
    if let Some(persistence_path) = persistence_path {
        app.add_plugins(WindowManagerPlugin::with_path(persistence_path));
    } else {
        app.add_plugins(WindowManagerPlugin);
    }
    app.add_plugins(remote::plugin())
        .add_plugins(remote::http_plugin())
        .add_observer(setup::on_spawn_managed_window)
        .add_observer(events::on_window_restored)
        .add_observer(events::on_window_restore_mismatch)
        .add_observer(setup::on_secondary_window_added)
        .add_observer(setup::on_secondary_window_removed)
        .add_observer(mode_observers::on_set_borderless_fullscreen)
        .add_observer(mode_observers::on_set_windowed)
        .add_observer(mode_observers::on_set_exclusive_fullscreen)
        .add_observer(mode_observers::on_toggle_persistence)
        .add_observer(mode_observers::on_clear_state_and_quit)
        .add_observer(mode_observers::on_quit_app)
        .insert_resource(KeyboardInputMode::from(
            var(TEST_MODE_ENVIRONMENT_VARIABLE).is_err(),
        ))
        .init_resource::<SelectedVideoModes>()
        .init_resource::<WindowCounter>()
        .init_resource::<RestoredStates>()
        .init_resource::<MismatchStates>()
        .init_resource::<WindowsSettledCount>()
        .add_systems(Startup, (setup::setup, debug::log_monitor_ids))
        .add_systems(
            Update,
            (
                display::update_primary_display,
                display::update_secondary_displays,
                input::handle_global_input.run_if(input::keyboard_enabled),
                input::handle_window_mode_input.run_if(input::keyboard_enabled),
                debug::debug_winit_monitor,
                debug::debug_window_changed,
                debug::debug_scale_factor_changed,
            ),
        )
        .run();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_launch_monitor_keeps_automatic_positioning() {
        // `io::Error` is not `PartialEq`, so unwrap and compare the `WindowPosition` itself.
        assert_eq!(
            parse_launch_position(None, None).unwrap(),
            WindowPosition::Automatic,
        );
    }

    #[test]
    fn launch_monitor_centers_the_initial_window_on_that_monitor() {
        assert_eq!(
            parse_launch_position(Some("2"), None).unwrap(),
            WindowPosition::Centered(MonitorSelection::Index(2)),
        );
    }

    #[test]
    fn explicit_launch_position_takes_precedence_over_monitor_centering() {
        assert_eq!(
            parse_launch_position(Some("2"), Some("-1200,80")).unwrap(),
            WindowPosition::At(IVec2::new(-1200, 80)),
        );
    }

    #[test]
    fn absent_launch_size_keeps_default_resolution() {
        assert!(parse_launch_size(None).unwrap().is_none());
    }

    #[test]
    fn explicit_launch_size_sets_the_window_resolution() {
        let resolution = parse_launch_size(Some("640,480")).unwrap().unwrap();
        assert_eq!(resolution.physical_width(), 640);
        assert_eq!(resolution.physical_height(), 480);
    }

    #[test]
    fn malformed_launch_size_is_an_error() {
        assert!(parse_launch_size(Some("640")).is_err());
        assert!(parse_launch_size(Some("640,abc")).is_err());
    }
}
