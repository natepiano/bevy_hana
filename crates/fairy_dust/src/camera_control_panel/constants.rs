//! Constants for the `camera_control_panel` module.

use std::time::Duration;

use bevy::prelude::Color;
use hana_diegetic::Px;

use crate::constants::LABEL_SIZE;

// colors
pub(super) const ACTIVE_COLOR: Color = Color::srgb(1.0, 0.9, 0.25);
pub(super) const HEADER_COLOR: Color = Color::srgb(0.3, 1.0, 0.8);
pub(super) const LABEL_COLOR: Color = Color::srgba(0.6, 0.65, 0.8, 0.85);

// table
/// Sized so the longest action label (`Zoom Out`) shares one column with the
/// shorter labels.
pub(super) const ACTION_COLUMN_WIDTH: Px = Px(70.0);
pub(super) const CONNECTOR_CAP_SIZE: Px = Px(5.0);
pub(super) const CONNECTOR_LEVEL_EPSILON: Px = Px(0.5);
pub(super) const CONNECTOR_LINE_WIDTH: Px = Px(1.0);
pub(super) const FEEDER_START_GAP: Px = Px(10.0);
/// Length of each horizontal segment of the worst-case connector, measured as
/// drawn line (the arrowhead is excluded). Chosen so the feeder line and the
/// cap-excluded trunk line are equal, placing the riser at the midpoint of the
/// equivalent straight line without widening the panel.
pub(super) const MIN_CONNECTOR_HORIZONTAL: Px = Px(11.5);
pub(super) const TRUNK_END_GAP: Px = Px(8.0);
pub(super) const FEEDER_CELL_MIN: Px = Px(FEEDER_START_GAP.0 + MIN_CONNECTOR_HORIZONTAL.0);
/// The trunk reserves the arrowhead (`CONNECTOR_CAP_SIZE`) on top of its drawn
/// line, so its plain segment equals `MIN_CONNECTOR_HORIZONTAL`.
pub(super) const SPACER_WIDTH: Px =
    Px(MIN_CONNECTOR_HORIZONTAL.0 + CONNECTOR_CAP_SIZE.0 + TRUNK_END_GAP.0);
pub(super) const GUIDANCE_CHILD_GAP: Px = Px(5.0);
pub(super) const HIGHLIGHT_RELEASE_HOLD: Duration = Duration::from_millis(200);
/// Home-row highlight window after a home tap. The eased glide to the home pose
/// has no discrete end, so the row stays lit for this fixed span — sized to
/// cover a typical fly-home rather than a shorter interaction fade — to read as
/// "flying home" before it fades.
pub(super) const HOME_HIGHLIGHT_HOLD: Duration = Duration::from_millis(1500);
pub(super) const LABEL_LINE_HEIGHT: Px = Px(LABEL_SIZE.0 + 5.0);
/// Width of the `Normal` / `Slow` speed column, sized so both share one column
/// and the binding column starts at the same x in each block.
pub(super) const SPEED_LABEL_COLUMN_WIDTH: Px = Px(56.0);
pub(super) const TABLE_COLUMN_GAP: f32 = 8.0;
pub(super) const TABLE_DIVIDER_WIDTH: Px = Px(1.0);
pub(super) const TABLE_GROUP_GAP: f32 = 7.0;
pub(super) const TABLE_ROW_GAP: f32 = 3.0;
pub(super) const TABLE_SECTION_DIVIDER_GAP: f32 = 13.0;
