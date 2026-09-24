//! Restart on Unix: `exec` replaces the running example with cargo, so Bevy shutdown never runs.

use std::io::Error;
use std::os::unix::process::CommandExt;
use std::process::Command;

/// Replaces this process with `command`; returns only when `exec` fails.
pub(super) fn hand_off(command: &mut Command) -> Error { command.exec() }
