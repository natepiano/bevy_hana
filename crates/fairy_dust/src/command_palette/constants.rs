//! Command palette geometry, colors, and fixed text.

use bevy::prelude::Color;

// command rows
//
// Every row is one line of unwrapped text in a fixed-height box. The `MAX_CHARS` budgets
// keep each column's text inside its own box: unwrapped text overflows rather than
// wrapping, and an overflowing column would print over its neighbor.
pub(super) const COMMAND_KEYSTROKE_COLOR: Color = Color::srgb(0.58, 0.63, 0.66);
pub(super) const COMMAND_KEYSTROKE_COLUMN_WIDTH: f32 = 180.0;
pub(super) const COMMAND_KEYSTROKE_MAX_CHARS: usize = 24;
pub(super) const COMMAND_ROW_HEIGHT: f32 = 17.0;
pub(super) const COMMAND_TEXT_SIZE: f32 = 12.0;
pub(super) const COMMAND_TITLE_COLOR: Color = Color::srgb(0.82, 0.88, 0.92);
pub(super) const COMMAND_TITLE_MAX_CHARS: usize = 56;
pub(super) const HIDDEN_ROW_COLOR: Color = Color::srgb(0.58, 0.63, 0.66);
/// Rows past this count are summarized by one trailing "more" row rather than
/// dropped without a trace.
pub(super) const MAX_VISIBLE_COMMAND_ROWS: usize = 12;
pub(super) const SELECTED_COMMAND_BACKGROUND: Color = Color::srgba(0.24, 0.42, 0.52, 0.55);
pub(super) const SELECTED_COMMAND_TITLE_COLOR: Color = Color::srgb(0.96, 0.99, 1.0);

// failure rows
pub(super) const FAILURE_ACTION_COLUMN_WIDTH: f32 = 62.0;
pub(super) const FAILURE_ADVISORY_COLOR: Color = Color::srgb(0.87, 0.78, 0.45);
pub(super) const FAILURE_COLOR: Color = Color::srgb(1.0, 0.48, 0.36);
pub(super) const FAILURE_LINE_MAX_CHARS: usize = 88;
pub(super) const FAILURE_ROW_HEIGHT: f32 = 15.0;
pub(super) const FAILURE_TEXT_SIZE: f32 = 11.0;
/// Prefix of the panel-local id every clickable failure-row action carries. The
/// row's index follows, which is what the click observer resolves back to an
/// action.
pub(super) const FAILURE_ACTION_ID_PREFIX: &str = "keymap-failure-action-";

// query field
pub(super) const FIELD_BACKGROUND: Color = Color::srgba(0.030, 0.038, 0.044, 0.94);
pub(super) const FIELD_BORDER: Color = Color::srgba(0.42, 0.72, 0.86, 0.82);
pub(super) const FIELD_BORDER_WIDTH: f32 = 1.0;
pub(super) const FIELD_CORNER_RADIUS: f32 = 4.0;
pub(super) const FIELD_HEIGHT: f32 = 38.0;
pub(super) const FIELD_ID: &str = "query";
pub(super) const FIELD_PADDING_X: f32 = 8.0;
pub(super) const FIELD_PADDING_Y: f32 = 6.0;
/// Fill behind the characters the editor has selected, which is the whole query
/// while the palette is first opened.
pub(super) const FIELD_SELECTION_COLOR: Color = Color::srgba(0.42, 0.72, 0.86, 0.35);
pub(super) const FIELD_TEXT_COLOR: Color = Color::srgb(0.88, 0.95, 0.92);
pub(super) const FIELD_TEXT_SIZE: f32 = 14.0;
pub(super) const PLACEHOLDER_COLOR: Color = Color::srgb(0.48, 0.54, 0.58);
pub(super) const PLACEHOLDER_TEXT: &str = "Type a command title or id";

// panel frame
pub(super) const PANEL_BACKGROUND: Color = Color::srgba(0.055, 0.068, 0.078, 0.96);
pub(super) const PANEL_BORDER: Color = Color::srgba(0.34, 0.52, 0.62, 0.85);
pub(super) const PANEL_BORDER_WIDTH: f32 = 1.0;
pub(super) const PANEL_COLUMN_GAP: f32 = 8.0;
pub(super) const PANEL_CORNER_RADIUS: f32 = 6.0;
/// Window pixels left between the palette box and each window edge once the
/// window is too narrow for [`PANEL_MAX_WIDTH`].
pub(super) const PANEL_EDGE_MARGIN: f32 = 24.0;
/// Width the box holds while the window is wide enough for it; below that the
/// box shrinks with the window rather than hanging off the left edge.
pub(super) const PANEL_MAX_WIDTH: f32 = 900.0;
/// Narrowest the box shrinks to, so the query field never collapses.
pub(super) const PANEL_MIN_WIDTH: f32 = 320.0;
pub(super) const PANEL_PADDING: f32 = 10.0;
pub(super) const PANEL_ROW_GAP: f32 = 4.0;
/// Fraction of the window height the palette's top edge sits at, leaving the
/// scene visible above and below the box.
pub(super) const PANEL_TOP_RATIO: f32 = 0.14;
pub(super) const SEPARATOR_COLOR: Color = Color::srgba(0.30, 0.38, 0.44, 0.7);
pub(super) const SEPARATOR_HEIGHT: f32 = 1.0;
