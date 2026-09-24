//! Native window behavior on targets whose window system needs no tabbing fix and reports no
//! native fullscreen transitions. Every observation is `Unavailable`, and activation does
//! nothing.

use bevy::prelude::App;
use bevy::prelude::Entity;

use crate::restore::NativeFullscreenState;

pub(crate) fn register(app: &mut App) { app.insert_non_send(NativeFullscreenObservations); }

/// Native fullscreen observations on a target that reports none.
pub(crate) struct NativeFullscreenObservations;

#[allow(
    clippy::needless_pass_by_ref_mut,
    clippy::unused_self,
    reason = "the macOS backend's observation table is the receiver; this backend keeps its \
              signatures so the restore code calls one API"
)]
impl NativeFullscreenObservations {
    pub(crate) const fn observe(&mut self, _: Entity) -> NativeFullscreenState {
        NativeFullscreenState::Unavailable
    }

    pub(crate) const fn stop(&mut self, _: Entity) {}
}

pub(crate) const fn activate_fullscreen_window(_: Entity) {}
