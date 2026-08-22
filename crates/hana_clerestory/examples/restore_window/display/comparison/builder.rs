use bevy::prelude::ChildSpawnerCommands;
use bevy::prelude::TextFont;
use bevy::prelude::Window;
use hana_clerestory::CurrentMonitor;

use super::add_span;
use super::rows;
use super::values::CurrentValues;
use crate::constants::DEFAULT_COLOR;
use crate::constants::EFFECTIVE_MODE_LABEL;
use crate::events::CachedMismatchState;
use crate::events::CachedRestoredState;

pub(super) fn build_comparison_spans(
    child_spawner: &mut ChildSpawnerCommands,
    cached_restored_state: Option<&CachedRestoredState>,
    cached_mismatch_state: Option<&CachedMismatchState>,
    window: &Window,
    current_monitor: &CurrentMonitor,
    text_font: &TextFont,
) {
    let effective_window_mode = current_monitor.effective_window_mode;
    let current_values = CurrentValues::from_window(window, current_monitor);

    if let Some(cached_restored_state) = cached_restored_state {
        rows::build_restored_spans(
            child_spawner,
            cached_restored_state,
            cached_mismatch_state,
            &current_values,
            text_font,
        );
    } else {
        rows::build_current_only_spans(child_spawner, &current_values, text_font);
    }

    add_span(
        child_spawner,
        text_font,
        &format!("\n{EFFECTIVE_MODE_LABEL} {effective_window_mode:?}\n"),
        DEFAULT_COLOR,
    );
}
