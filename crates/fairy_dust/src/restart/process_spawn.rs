//! Restart on targets without `exec`, Windows among them: cargo is spawned as a child process and
//! this process exits.

use std::io::Error;
use std::process;
use std::process::Command;

use bevy::prelude::info;

use super::constants::EXIT_STATUS_SUCCESS;

/// Spawns `command` and exits this process; returns only when the spawn fails.
pub(super) fn hand_off(command: &mut Command) -> Error {
    match command.spawn() {
        Ok(child) => {
            info!("fairy_dust restart: cargo spawned as pid {}", child.id());
            process::exit(EXIT_STATUS_SUCCESS);
        },
        Err(err) => err,
    }
}
