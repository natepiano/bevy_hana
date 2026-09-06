use bevy::prelude::Query;
use bevy::prelude::With;

use super::target::TargetPosition;
use crate::restore::WindowRestoreAttempt;

/// Whether any entity carries both `TargetPosition` and `WindowRestoreAttempt`, which is a window
/// with a restore in progress.
pub(crate) fn has_restoring_windows(
    query: Query<(), (With<TargetPosition>, With<WindowRestoreAttempt>)>,
) -> bool {
    !query.is_empty()
}
