use bevy::prelude::Query;
use bevy::prelude::With;

use super::target::TargetPosition;
use crate::restore::RestorePreparation;

/// Run condition: returns true if any entity has a `TargetPosition` component.
pub(crate) fn has_restoring_windows(
    query: Query<(), (With<TargetPosition>, With<RestorePreparation>)>,
) -> bool {
    !query.is_empty()
}
