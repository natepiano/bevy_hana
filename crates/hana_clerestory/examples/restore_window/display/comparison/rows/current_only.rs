use bevy::prelude::ChildSpawnerCommands;
use bevy::prelude::TextFont;

use super::super::values::CurrentValues;
use super::span::add_span;
use crate::constants::DEFAULT_COLOR;
use crate::constants::LABEL_WIDTH;
use crate::constants::MISMATCH_COLOR;
use crate::constants::MODE_LABEL;
use crate::constants::MONITOR_INDEX_LABEL;
use crate::constants::NO_RESTORE_DATA_TEXT;
use crate::constants::POSITION_LOGICAL_LABEL;
use crate::constants::POSITION_PHYSICAL_LABEL;
use crate::constants::SCALE_LABEL;
use crate::constants::SIZE_LOGICAL_LABEL;
use crate::constants::SIZE_PHYSICAL_LABEL;

/// Render current-only values when no restore data exists.
pub(super) fn build_current_only_spans(
    child_spawner: &mut ChildSpawnerCommands,
    current_values: &CurrentValues,
    text_font: &TextFont,
) {
    add_span(
        child_spawner,
        text_font,
        NO_RESTORE_DATA_TEXT,
        MISMATCH_COLOR,
    );
    add_span(
        child_spawner,
        text_font,
        &format!(
            "{:<LABEL_WIDTH$}{}\n",
            POSITION_PHYSICAL_LABEL, current_values.physical_position
        ),
        DEFAULT_COLOR,
    );
    add_span(
        child_spawner,
        text_font,
        &format!(
            "{:<LABEL_WIDTH$}{}\n",
            POSITION_LOGICAL_LABEL, current_values.logical_position
        ),
        DEFAULT_COLOR,
    );
    add_span(
        child_spawner,
        text_font,
        &format!(
            "{:<LABEL_WIDTH$}{}\n",
            SIZE_PHYSICAL_LABEL, current_values.physical_size
        ),
        DEFAULT_COLOR,
    );
    add_span(
        child_spawner,
        text_font,
        &format!(
            "{:<LABEL_WIDTH$}{}\n",
            SIZE_LOGICAL_LABEL, current_values.logical_size
        ),
        DEFAULT_COLOR,
    );
    add_span(
        child_spawner,
        text_font,
        &format!("{:<LABEL_WIDTH$}{}\n", SCALE_LABEL, current_values.scale),
        DEFAULT_COLOR,
    );
    add_span(
        child_spawner,
        text_font,
        &format!(
            "{:<LABEL_WIDTH$}{}\n",
            MONITOR_INDEX_LABEL, current_values.monitor
        ),
        DEFAULT_COLOR,
    );
    add_span(
        child_spawner,
        text_font,
        &format!("{:<LABEL_WIDTH$}{}\n", MODE_LABEL, current_values.mode),
        DEFAULT_COLOR,
    );
}
