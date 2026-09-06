//! Capability: Ctrl+Shift+R rebuilds and re-launches the example via cargo.
//!
//! Declared with `hana_rubric`'s `command!`, so the keymap owns the chord and
//! the palette lists it. The observing system
//! relaunches the example as `cargo run --manifest-path <workspace>/Cargo.toml
//! --example <name>`.
//!
//! `AppExit` is bypassed because on macOS the winit run loop can fail to honor
//! `AppExit::Success` cleanly, leaving the old window stuck. Unix builds
//! use `exec`, so the current Bevy process is replaced by cargo without running
//! Bevy shutdown. Windows keeps a spawn-and-exit path.
//!
//! The example name is derived from `current_exe()`: cargo writes example
//! binaries to `<target>/<profile>/examples/<name>`. The manifest path is
//! derived from Fairy Dust's source location, not from the target directory, so
//! restart keeps working when a developer uses a shared or foreign
//! `CARGO_TARGET_DIR`.

#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::path::PathBuf;
use std::process;
use std::process::Command;

use bevy::prelude::App;
use bevy::prelude::Event;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::Res;
use bevy::prelude::With;
use bevy::prelude::info;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::InputAction;
use hana_lagrange::OrbitCam;
use hana_rubric::ReflectKeymapCommand;
use hana_rubric::command;

use crate::constants::CARGO_BIN;
use crate::constants::CARGO_EXAMPLE_FLAG;
use crate::constants::CARGO_EXAMPLES_DIR;
use crate::constants::CARGO_MANIFEST_PATH_FLAG;
use crate::constants::CARGO_RELEASE_FLAG;
use crate::constants::CARGO_RUN_SUBCOMMAND;
use crate::constants::EXIT_STATUS_FAILURE;
#[cfg(windows)]
use crate::constants::EXIT_STATUS_SUCCESS;
use crate::ensure_plugin;
use crate::orbit_cam::FairyDustOrbitCam;
use crate::restart_camera;
use crate::restart_camera::RestartCameraRestore;

command! {
    action:      Restart,
    event:       RestartEvent,
    id:          "fairy_dust::restart",
    title:       "Restart Example",
    description: "Exit and relaunch this example through cargo, rebuilding any source edits.",
}

pub(crate) fn install(app: &mut App) {
    ensure_plugin(app, EnhancedInputPlugin);
    app.add_observer(request_restart);
}

fn request_restart(
    _: On<RestartEvent>,
    cameras: Query<&OrbitCam, With<FairyDustOrbitCam>>,
    restore_state: Option<Res<RestartCameraRestore>>,
) {
    info!("fairy_dust restart: Ctrl+Shift+R pressed, invoking cargo");
    let encoded_pose = restart_camera::encode_child_pose(&cameras, restore_state.as_deref());
    do_restart(encoded_pose);
}

/// Does nothing: restart leaves the current process from the input handler.
/// Kept so [`crate::SprinkleBuilder::run`] needs no cfg branch.
pub(crate) const fn perform_restart_if_requested() {}

#[cfg(unix)]
fn do_restart(encoded_pose: Option<String>) {
    let mut command = restart_command(encoded_pose);
    let err = command.exec();
    eprintln!("fairy_dust: failed to exec `cargo run`: {err}");
    process::exit(EXIT_STATUS_FAILURE);
}

#[cfg(windows)]
fn do_restart(encoded_pose: Option<String>) {
    let mut command = restart_command(encoded_pose);
    match command.spawn() {
        Ok(child) => {
            info!("fairy_dust restart: cargo spawned as pid {}", child.id());
            process::exit(EXIT_STATUS_SUCCESS);
        },
        Err(err) => {
            eprintln!("fairy_dust: failed to spawn `cargo run`: {err}");
            process::exit(EXIT_STATUS_FAILURE);
        },
    }
}

#[cfg(any(unix, windows))]
fn restart_command(encoded_pose: Option<String>) -> Command {
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("fairy_dust: current_exe failed: {err}");
            process::exit(EXIT_STATUS_FAILURE);
        },
    };
    let Some(example_name) = derive_example_name(&exe) else {
        eprintln!(
            "fairy_dust: could not derive cargo example context from {}; \
             restart requires the binary to live at \
             <target>/<profile>/examples/<name>",
            exe.display(),
        );
        process::exit(EXIT_STATUS_FAILURE);
    };
    let Some(manifest_path) = source_workspace_manifest() else {
        eprintln!(
            "fairy_dust: could not derive source workspace Cargo.toml from {}",
            env!("CARGO_MANIFEST_DIR"),
        );
        process::exit(EXIT_STATUS_FAILURE);
    };
    let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    // Re-launch in the profile the running binary was built with: a release
    // build has `debug_assertions` off, so pass `--release`; a debug build uses
    // the default dev profile.
    let release = !cfg!(debug_assertions);
    let release_fragment = if release {
        format!(" {CARGO_RELEASE_FLAG}")
    } else {
        String::default()
    };
    info!(
        "fairy_dust restart: launching `cargo run{release_fragment} --manifest-path {} --example {}` in {}",
        manifest_path.display(),
        example_name,
        manifest_dir.display(),
    );
    let mut command = Command::new(CARGO_BIN);
    command.arg(CARGO_RUN_SUBCOMMAND);
    if release {
        command.arg(CARGO_RELEASE_FLAG);
    }
    command
        .arg(CARGO_MANIFEST_PATH_FLAG)
        .arg(&manifest_path)
        .args([CARGO_EXAMPLE_FLAG, &example_name])
        .current_dir(manifest_dir);
    restart_camera::apply_child_env(&mut command, encoded_pose);
    command
}

#[cfg(not(any(unix, windows)))]
fn do_restart(_: Option<String>) {
    eprintln!("fairy_dust: restart not supported on this platform");
}

/// Recovers the example name from the running binary's path.
///
/// Returns `None` unless the path has the layout cargo produces for examples:
/// `<target>/<profile>/examples/<name>`.
#[cfg(any(unix, windows))]
fn derive_example_name(exe: &Path) -> Option<String> {
    let name = exe.file_name()?.to_str()?.to_string();
    let examples_dir = exe.parent()?;
    if examples_dir.file_name()?.to_str()? != CARGO_EXAMPLES_DIR {
        return None;
    }
    let profile_dir = examples_dir.parent()?;
    let _target_dir = profile_dir.parent()?;
    Some(name)
}

#[cfg(any(unix, windows))]
fn source_workspace_manifest() -> Option<PathBuf> {
    let package_manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = package_manifest.parent()?.parent()?;
    Some(workspace_root.join("Cargo.toml"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::derive_example_name;
    use super::source_workspace_manifest;

    #[test]
    fn example_name_comes_from_foreign_target_dir_without_workspace_root() {
        let exe = Path::new("/tmp/shared-target/debug/examples/batch_validation");

        assert_eq!(
            derive_example_name(exe),
            Some("batch_validation".to_string())
        );
    }

    #[test]
    fn source_workspace_manifest_points_at_this_workspace() {
        assert!(
            source_workspace_manifest()
                .as_ref()
                .is_some_and(|manifest| manifest.ends_with("Cargo.toml") && manifest.exists())
        );
    }
}
