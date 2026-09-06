//! Window-only recovery behavior layered on the rigging kernel.

mod fallback_and_return;

use bevy::prelude::App;
use bevy::prelude::Plugin;
pub(crate) use fallback_and_return::StrandedWindowDisplayObservation;
pub(crate) use fallback_and_return::StrandedWindowMovementBaselines;
pub(crate) use fallback_and_return::StrandedWindowObservation;
#[cfg(test)]
pub(crate) use fallback_and_return::WindowFallbackRecoveryPhase;
#[cfg(test)]
pub(crate) use fallback_and_return::WindowFallbackRecoveryProgress;
pub(crate) use fallback_and_return::WindowFallbackRecoveryState;

/// Owns the recovery bookkeeping. The systems that read it live with the window driver, which is
/// what supplies the monitor, platform, and registry resources they need.
pub(crate) struct RecoveryPlugin;

impl Plugin for RecoveryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WindowFallbackRecoveryState>()
            .init_resource::<StrandedWindowMovementBaselines>()
            .add_observer(fallback_and_return::on_retire_role);
    }
}
