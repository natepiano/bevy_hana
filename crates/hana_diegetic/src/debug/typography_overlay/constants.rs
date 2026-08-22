//! Typography overlay-specific defaults.

// dimension arrow stack slots
/// Slot index (in `arrow_spacing` units) for the outermost left dimension arrow
/// (ascent-to-descent), measured from the first glyph's left edge.
pub(super) const LEFT_OUTER_ARROW_SLOT: f32 = 3.0;
/// Total metric-rectangle outer width in `arrow_spacing` units beyond the text run.
pub(super) const METRIC_RECT_WIDTH_SLOTS: f32 = 5.0;
/// Slot index (in `arrow_spacing` units) for the outermost right dimension arrow
/// (cap-height), measured from the last glyph's right edge.
pub(super) const RIGHT_OUTER_ARROW_SLOT: f32 = 2.0;

// overlay defaults
/// Default extension distance for overlay annotation lines, in layout units.
pub(super) const DEFAULT_OVERLAY_EXTEND: f32 = 8.0;
/// Default font size for overlay metric labels.
pub(super) const DEFAULT_OVERLAY_LABEL_SIZE: f32 = 6.0;

// overlay entities
/// Bevy `Name` assigned to the hidden overlay-bounds target entity.
pub(super) const OVERLAY_BOUNDING_BOX_NAME: &str = "OverlayBoundingBox";

// overlay labels
/// Prefix used for "no line gap" labels on overlay annotations.
pub(super) const NO_LINE_GAP_LABEL_PREFIX: &str = "no line gap for ";
