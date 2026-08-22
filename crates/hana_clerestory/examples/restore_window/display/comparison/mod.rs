mod builder;
mod rows;
mod values;

use bevy::prelude::ChildSpawnerCommands;
use bevy::prelude::Color;
use bevy::prelude::TextFont;
use bevy::prelude::Window;
use hana_clerestory::CurrentMonitor;

use crate::events::CachedMismatchState;
use crate::events::CachedRestoredState;

/// Build comparison spans (restored vs current) for a window and add them as `TextSpan` children.
pub(super) fn build_comparison_spans(
    child_spawner: &mut ChildSpawnerCommands,
    cached_restored_state: Option<&CachedRestoredState>,
    cached_mismatch_state: Option<&CachedMismatchState>,
    window: &Window,
    current_monitor: &CurrentMonitor,
    text_font: &TextFont,
) {
    builder::build_comparison_spans(
        child_spawner,
        cached_restored_state,
        cached_mismatch_state,
        window,
        current_monitor,
        text_font,
    );
}

/// Add a single `TextSpan` child.
pub(super) fn add_span(
    child_spawner: &mut ChildSpawnerCommands,
    text_font: &TextFont,
    text: &str,
    color: Color,
) {
    rows::add_span(child_spawner, text_font, text, color);
}
