//! Typography overlay-specific defaults.

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
pub(super) const NO_LINE_GAP_LABEL_PREFIX: &str = "No line gap for ";
