//! Output proof on targets with no native backend yet: no reading of whether a window is on
//! screen is available, so the signal is absent and no native window is ever observed.

use bevy::prelude::Entity;
use hana_rigging::prelude::ConfirmationSignal;

use super::WindowOnScreenObservation;

/// No native report of whether a window is on screen exists here.
pub(super) const SIGNAL: ConfirmationSignal = ConfirmationSignal::Absent;

pub(super) const fn observe_native_window_on_screen(_: Entity) -> WindowOnScreenObservation {
    WindowOnScreenObservation::NoNativeWindowYet
}
