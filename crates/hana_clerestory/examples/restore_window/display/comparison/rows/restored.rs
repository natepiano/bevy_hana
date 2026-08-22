use bevy::prelude::ChildSpawnerCommands;
use bevy::prelude::TextFont;

use super::super::values::CurrentValues;
use super::super::values::RestoredValues;
use super::span::add_span;
use crate::constants::ACTUAL_COLUMN_TITLE;
use crate::constants::CURRENT_COLUMN_TITLE;
use crate::constants::DEFAULT_COLOR;
use crate::constants::EXPECTED_COLUMN_TITLE;
use crate::constants::LABEL_WIDTH;
use crate::constants::MIN_COMPARISON_COLUMN_WIDTH;
use crate::constants::MISMATCH_COLOR;
use crate::constants::MISMATCH_WARN_COLOR;
use crate::constants::MODE_LABEL;
use crate::constants::MONITOR_INDEX_LABEL;
use crate::constants::NONE_TEXT;
use crate::constants::POSITION_LOGICAL_LABEL;
use crate::constants::POSITION_PHYSICAL_LABEL;
use crate::constants::RESTORED_COLUMN_TITLE;
use crate::constants::SCALE_LABEL;
use crate::constants::SIZE_LOGICAL_LABEL;
use crate::constants::SIZE_PHYSICAL_LABEL;
use crate::events::CachedMismatchState;
use crate::events::CachedRestoredState;

struct ComparisonMismatch {
    expected: String,
    actual:   String,
}

struct ComparisonRow<'a> {
    label:    &'a str,
    restored: String,
    current:  String,
    mismatch: Option<ComparisonMismatch>,
}

#[derive(Clone, Copy)]
enum ComparisonLayout {
    CurrentOnly,
    WithMismatchColumns,
}

/// Render comparison rows when restore data is available.
pub(super) fn build_restored_spans(
    child_spawner: &mut ChildSpawnerCommands,
    cached_restored_state: &CachedRestoredState,
    cached_mismatch_state: Option<&CachedMismatchState>,
    current_values: &CurrentValues,
    text_font: &TextFont,
) {
    let restored_values = RestoredValues::from(cached_restored_state);
    let column_width = restored_values
        .comparison_width()
        .max(MIN_COMPARISON_COLUMN_WIDTH);
    let comparison_layout = if cached_mismatch_state.is_some() {
        ComparisonLayout::WithMismatchColumns
    } else {
        ComparisonLayout::CurrentOnly
    };

    add_restored_header(child_spawner, text_font, comparison_layout, column_width);
    add_position_rows(
        child_spawner,
        text_font,
        &restored_values,
        current_values,
        cached_mismatch_state,
        column_width,
    );
    add_size_rows(
        child_spawner,
        text_font,
        &restored_values,
        current_values,
        cached_mismatch_state,
        column_width,
    );
    add_scale_row(
        child_spawner,
        text_font,
        current_values,
        cached_mismatch_state,
        column_width,
    );
    add_monitor_row(
        child_spawner,
        text_font,
        &restored_values,
        current_values,
        cached_mismatch_state,
        column_width,
    );
    add_mode_row(
        child_spawner,
        text_font,
        &restored_values,
        current_values,
        cached_mismatch_state,
        column_width,
    );
}

fn add_restored_header(
    child_spawner: &mut ChildSpawnerCommands,
    text_font: &TextFont,
    comparison_layout: ComparisonLayout,
    column_width: usize,
) {
    let header = if matches!(comparison_layout, ComparisonLayout::WithMismatchColumns) {
        format!(
            "{:LABEL_WIDTH$}{:<column_width$}{:<column_width$}{:<column_width$}{}\n",
            "",
            RESTORED_COLUMN_TITLE,
            CURRENT_COLUMN_TITLE,
            EXPECTED_COLUMN_TITLE,
            ACTUAL_COLUMN_TITLE
        )
    } else {
        format!(
            "{:LABEL_WIDTH$}{:<column_width$}{}\n",
            "", RESTORED_COLUMN_TITLE, CURRENT_COLUMN_TITLE
        )
    };
    add_span(child_spawner, text_font, &header, DEFAULT_COLOR);
}

fn add_position_rows(
    child_spawner: &mut ChildSpawnerCommands,
    text_font: &TextFont,
    restored_values: &RestoredValues,
    current_values: &CurrentValues,
    cached_mismatch_state: Option<&CachedMismatchState>,
    column_width: usize,
) {
    add_row(
        child_spawner,
        text_font,
        &ComparisonRow {
            label:    POSITION_PHYSICAL_LABEL,
            restored: restored_values.physical_position.clone(),
            current:  current_values.physical_position.clone(),
            mismatch: cached_mismatch_state.map(|cached_mismatch_state| ComparisonMismatch {
                expected: cached_mismatch_state
                    .physical_position_mismatch
                    .expected
                    .map_or_else(
                        || NONE_TEXT.to_string(),
                        |position| format!("({}, {})", position.x, position.y),
                    ),
                actual:   cached_mismatch_state
                    .physical_position_mismatch
                    .actual
                    .map_or_else(
                        || NONE_TEXT.to_string(),
                        |position| format!("({}, {})", position.x, position.y),
                    ),
            }),
        },
        column_width,
    );
    add_row(
        child_spawner,
        text_font,
        &ComparisonRow {
            label:    POSITION_LOGICAL_LABEL,
            restored: restored_values.logical_position.clone(),
            current:  current_values.logical_position.clone(),
            mismatch: cached_mismatch_state.map(|cached_mismatch_state| ComparisonMismatch {
                expected: cached_mismatch_state
                    .logical_position_mismatch
                    .expected
                    .map_or_else(
                        || NONE_TEXT.to_string(),
                        |position| format!("({}, {})", position.x, position.y),
                    ),
                actual:   cached_mismatch_state
                    .logical_position_mismatch
                    .actual
                    .map_or_else(
                        || NONE_TEXT.to_string(),
                        |position| format!("({}, {})", position.x, position.y),
                    ),
            }),
        },
        column_width,
    );
}

fn add_size_rows(
    child_spawner: &mut ChildSpawnerCommands,
    text_font: &TextFont,
    restored_values: &RestoredValues,
    current_values: &CurrentValues,
    cached_mismatch_state: Option<&CachedMismatchState>,
    column_width: usize,
) {
    add_row(
        child_spawner,
        text_font,
        &ComparisonRow {
            label:    SIZE_PHYSICAL_LABEL,
            restored: restored_values.physical_size.clone(),
            current:  current_values.physical_size.clone(),
            mismatch: cached_mismatch_state.map(|cached_mismatch_state| ComparisonMismatch {
                expected: format!(
                    "{}x{}",
                    cached_mismatch_state.physical_size_mismatch.expected.x,
                    cached_mismatch_state.physical_size_mismatch.expected.y
                ),
                actual:   format!(
                    "{}x{}",
                    cached_mismatch_state.physical_size_mismatch.actual.x,
                    cached_mismatch_state.physical_size_mismatch.actual.y
                ),
            }),
        },
        column_width,
    );
    add_row(
        child_spawner,
        text_font,
        &ComparisonRow {
            label:    SIZE_LOGICAL_LABEL,
            restored: restored_values.logical_size.clone(),
            current:  current_values.logical_size.clone(),
            mismatch: cached_mismatch_state.map(|cached_mismatch_state| ComparisonMismatch {
                expected: format!(
                    "{}x{}",
                    cached_mismatch_state.logical_size_mismatch.expected.x,
                    cached_mismatch_state.logical_size_mismatch.expected.y
                ),
                actual:   format!(
                    "{}x{}",
                    cached_mismatch_state.logical_size_mismatch.actual.x,
                    cached_mismatch_state.logical_size_mismatch.actual.y
                ),
            }),
        },
        column_width,
    );
}

fn add_scale_row(
    child_spawner: &mut ChildSpawnerCommands,
    text_font: &TextFont,
    current_values: &CurrentValues,
    cached_mismatch_state: Option<&CachedMismatchState>,
    column_width: usize,
) {
    if cached_mismatch_state.is_none() {
        add_span(
            child_spawner,
            text_font,
            &format!(
                "{:<LABEL_WIDTH$}{:<column_width$}{}\n",
                SCALE_LABEL, "", current_values.scale
            ),
            DEFAULT_COLOR,
        );
        return;
    }

    let comparison_row = ComparisonRow {
        label:    SCALE_LABEL,
        restored: String::new(),
        current:  current_values.scale.clone(),
        mismatch: cached_mismatch_state.map(|cached_mismatch_state| ComparisonMismatch {
            expected: cached_mismatch_state
                .scale_factor_difference
                .expected
                .to_string(),
            actual:   cached_mismatch_state
                .scale_factor_difference
                .actual
                .to_string(),
        }),
    };
    add_row(child_spawner, text_font, &comparison_row, column_width);
}

fn add_monitor_row(
    child_spawner: &mut ChildSpawnerCommands,
    text_font: &TextFont,
    restored_values: &RestoredValues,
    current_values: &CurrentValues,
    cached_mismatch_state: Option<&CachedMismatchState>,
    column_width: usize,
) {
    add_row(
        child_spawner,
        text_font,
        &ComparisonRow {
            label:    MONITOR_INDEX_LABEL,
            restored: restored_values.monitor.clone(),
            current:  current_values.monitor.clone(),
            mismatch: cached_mismatch_state.map(|cached_mismatch_state| ComparisonMismatch {
                expected: cached_mismatch_state
                    .monitor_difference
                    .expected
                    .to_string(),
                actual:   cached_mismatch_state.monitor_difference.actual.to_string(),
            }),
        },
        column_width,
    );
}

fn add_mode_row(
    child_spawner: &mut ChildSpawnerCommands,
    text_font: &TextFont,
    restored_values: &RestoredValues,
    current_values: &CurrentValues,
    cached_mismatch_state: Option<&CachedMismatchState>,
    column_width: usize,
) {
    add_row(
        child_spawner,
        text_font,
        &ComparisonRow {
            label:    MODE_LABEL,
            restored: restored_values.mode.clone(),
            current:  current_values.mode.clone(),
            mismatch: cached_mismatch_state.map(|cached_mismatch_state| ComparisonMismatch {
                expected: format!(
                    "{:?}",
                    cached_mismatch_state.window_mode_difference.expected
                ),
                actual:   format!("{:?}", cached_mismatch_state.window_mode_difference.actual),
            }),
        },
        column_width,
    );
}

/// Add a comparison row, dispatching to 3-column or 5-column layout based on mismatch data.
fn add_row(
    child_spawner: &mut ChildSpawnerCommands,
    text_font: &TextFont,
    comparison_row: &ComparisonRow<'_>,
    column_width: usize,
) {
    if let Some(comparison_mismatch) = comparison_row.mismatch.as_ref() {
        add_extended_comparison_row(
            child_spawner,
            text_font,
            comparison_row,
            comparison_mismatch,
            column_width,
        );
    } else {
        add_standard_comparison_row(child_spawner, text_font, comparison_row, column_width);
    }
}

/// Add a comparison row: label + file value (white) + current value (white or red if mismatch).
fn add_standard_comparison_row(
    child_spawner: &mut ChildSpawnerCommands,
    text_font: &TextFont,
    comparison_row: &ComparisonRow<'_>,
    column_width: usize,
) {
    let color = if comparison_row.restored == comparison_row.current {
        DEFAULT_COLOR
    } else {
        MISMATCH_COLOR
    };

    // `comparison_row.label` and `comparison_row.restored` use `DEFAULT_COLOR`.
    add_span(
        child_spawner,
        text_font,
        &format!(
            "{label:<LABEL_WIDTH$}{restored:<column_width$}",
            label = comparison_row.label,
            restored = comparison_row.restored
        ),
        DEFAULT_COLOR,
    );
    // `comparison_row.current` uses `MISMATCH_COLOR` when it differs from
    // `comparison_row.restored`.
    add_span(
        child_spawner,
        text_font,
        &format!("{}\n", comparison_row.current),
        color,
    );
}

/// Add a 5-column comparison row: label + restored + current + expected + actual.
/// Expected/actual columns use warning color when they differ.
fn add_extended_comparison_row(
    child_spawner: &mut ChildSpawnerCommands,
    text_font: &TextFont,
    comparison_row: &ComparisonRow<'_>,
    comparison_mismatch: &ComparisonMismatch,
    column_width: usize,
) {
    let current_color = if comparison_row.restored == comparison_row.current {
        DEFAULT_COLOR
    } else {
        MISMATCH_COLOR
    };
    let mismatch_color = if comparison_mismatch.expected == comparison_mismatch.actual {
        DEFAULT_COLOR
    } else {
        MISMATCH_WARN_COLOR
    };

    // `comparison_row.label` and `comparison_row.restored` use `DEFAULT_COLOR`.
    add_span(
        child_spawner,
        text_font,
        &format!(
            "{label:<LABEL_WIDTH$}{restored:<column_width$}",
            label = comparison_row.label,
            restored = comparison_row.restored
        ),
        DEFAULT_COLOR,
    );
    // `comparison_row.current` uses `MISMATCH_COLOR` when it differs from
    // `comparison_row.restored`.
    add_span(
        child_spawner,
        text_font,
        &format!("{current:<column_width$}", current = comparison_row.current),
        current_color,
    );
    // `comparison_mismatch.expected` uses `DEFAULT_COLOR`.
    add_span(
        child_spawner,
        text_font,
        &format!(
            "{expected:<column_width$}",
            expected = comparison_mismatch.expected
        ),
        DEFAULT_COLOR,
    );
    // `comparison_mismatch.actual` uses `MISMATCH_WARN_COLOR` on mismatch.
    add_span(
        child_spawner,
        text_font,
        &format!("{}\n", comparison_mismatch.actual),
        mismatch_color,
    );
}
