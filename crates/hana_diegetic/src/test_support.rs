use std::time::Duration;

use bevy::picking::PickingSettings;
use bevy::prelude::App;

/// Makes synthetic click counts independent of the time spent rendering frames.
pub(crate) fn configure_click_interval(app: &mut App, clicks: usize) {
    // Test gestures span frames; no CI stall should split a multi-click.
    const MULTI_CLICK_INTERVAL: Duration = Duration::MAX;
    let interval = if clicks > 1 {
        MULTI_CLICK_INTERVAL
    } else {
        Duration::ZERO
    };
    app.world_mut()
        .resource_mut::<PickingSettings>()
        .multi_click_interval = interval;
}
