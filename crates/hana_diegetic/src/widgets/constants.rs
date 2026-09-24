// slider pointer interaction
pub(super) const THUMB_CLICK_JITTER_PIXELS: f32 = 4.0;
pub(super) const THUMB_CLICK_JITTER_PIXELS_SQUARED: f32 =
    THUMB_CLICK_JITTER_PIXELS * THUMB_CLICK_JITTER_PIXELS;
pub(super) const THUMB_HIT_SLOP_POINTS: f32 = 4.0;

// wheel scrolling
/// Lines of a scrolling element's measured line pitch one wheel notch moves.
pub(super) const WHEEL_LINES_PER_NOTCH: f32 = 3.0;
