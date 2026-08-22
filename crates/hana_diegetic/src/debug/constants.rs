//! Constants for the typography overlay.

use bevy::color::Color;

// colors
/// Highlight color for the typographic baseline metric line and the
/// callouts that point at it. Reused for both the line itself and any
/// arrows or labels that visually anchor to the baseline.
pub(super) const BASELINE_COLOR: Color = Color::srgb(0.9, 0.2, 0.2);

/// Tint for per-glyph bounding boxes and the bounding-box callout — a
/// translucent warm yellow chosen to overlay glyph fills without
/// obscuring them.
pub(super) const BBOX_COLOR: Color = Color::srgba(1.0, 1.0, 0.6, 0.7);

// dimension ratios
/// Gap between arrow tips and metric lines relative to font size.
/// Multiplied by `font_size * LAYOUT_TO_WORLD` at usage to get world units.
pub(super) const ARROW_GAP_RATIO: f32 = 0.012;

/// Fraction of the first glyph's advance width used as the spacing
/// unit between arrow columns.
pub(super) const ARROW_SPACING_RATIO: f32 = 0.28;

/// Arrowhead line length relative to font size.
/// Multiplied by `font_size * LAYOUT_TO_WORLD` at usage to get world units.
pub(super) const ARROWHEAD_RATIO: f32 = 0.017;

/// Minimum world-unit thickness for bounding-box borders and callout
/// lines, expressed as a fraction of `font_size * scale`.
pub(super) const BBOX_MIN_WORLD_RATIO: f32 = 0.0025;

/// Length of each dash segment in the advancement bracket, expressed
/// as a fraction of the column spacing.
pub(super) const BRACKET_DASH_RATIO: f32 = 0.125;

/// Length of the gap between dash segments in the advancement
/// bracket, expressed as a fraction of the column spacing.
pub(super) const BRACKET_GAP_RATIO: f32 = BRACKET_DASH_RATIO / 2.0;

/// Radius of origin/advancement dots relative to font size.
/// Multiplied by `font_size * LAYOUT_TO_WORLD` at usage to get world units.
pub(super) const DOT_RADIUS_RATIO: f32 = 0.01;

/// Gap between labels and the elements they annotate, relative to font size.
/// Multiplied by `font_size * LAYOUT_TO_WORLD` at usage to get world units.
pub(super) const LABEL_GAP_RATIO: f32 = 0.02;

/// Font size for metric labels relative to the text's font size.
/// Apple's reference diagram uses labels roughly 1/10th the display size.
pub(super) const LABEL_SIZE_RATIO: f32 = 0.06;

// label strings
/// Label for the advancement dimension arrow.
pub(super) const LABEL_ADVANCEMENT: &str = "advancement";

/// Label for the ascent metric line and dimension arrow.
pub(super) const LABEL_ASCENT: &str = "ascent";

/// Label for the baseline metric line.
pub(super) const LABEL_BASELINE: &str = "baseline";

/// Label for the bottom metric line.
pub(super) const LABEL_BOTTOM: &str = "bottom";

/// Label for the bounding box callout.
pub(super) const LABEL_BOUNDING_BOX: &str = "bounding box";

/// Label for the cap height metric line and dimension arrow.
pub(super) const LABEL_CAP_HEIGHT: &str = "cap height";

/// Label for the descent metric line and dimension arrow.
pub(super) const LABEL_DESCENT: &str = "descent";

/// Label for the line height dimension arrow.
pub(super) const LABEL_LINE_HEIGHT: &str = "line height";

/// Label for the origin callout.
pub(super) const LABEL_ORIGIN: &str = "origin";

/// Label for the top metric line.
pub(super) const LABEL_TOP: &str = "top";

/// Label for the x-height metric line and dimension arrow.
pub(super) const LABEL_X_HEIGHT: &str = "x-height";

// line widths
/// Default line width for overlay guides (in pixels).
pub(super) const DEFAULT_LINE_WIDTH: f32 = 0.5;

/// Multiplier applied to `bbox_border_width` to derive the border
/// width for horizontal metric lines.
pub(super) const METRIC_LINE_BORDER_MULT: f32 = 1.5;

/// Line width for metric lines, bounding boxes, and callout backgrounds.
pub(super) const THIN_LINE_WIDTH: f32 = 1.0;

// z-layer offsets
/// Z offset for callout elements (bounding boxes, origin dots, advancement arrows).
pub(super) const CALLOUT_Z_OFFSET: f32 = 0.0;

/// Z offset for metric lines and vertical dimension arrows.
pub(super) const METRIC_LINE_Z_OFFSET: f32 = 0.0;
