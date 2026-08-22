//! Shared constants for the layout engine.

// font feature tags
/// OpenType tag for contextual alternates.
pub(super) const CALT_TAG: [u8; 4] = *b"calt";
/// OpenType tag for discretionary ligatures.
pub(super) const DLIG_TAG: [u8; 4] = *b"dlig";
/// OpenType tag for kerning.
pub(super) const KERN_TAG: [u8; 4] = *b"kern";
/// OpenType tag for standard ligatures.
pub(super) const LIGA_TAG: [u8; 4] = *b"liga";

// layout engine
/// Inline capacity for child index lists. Most elements have 1–4 children;
/// only top-level containers (e.g., a column of many rows) exceed this and
/// spill to the heap.
pub(super) const INLINE_CHILDREN: usize = 4;
/// Epsilon for layout convergence — differences below this are not visually
/// meaningful but can cause iterative sizing loops to spin forever.
pub(super) const LAYOUT_EPSILON: f32 = 0.01;

// text defaults
/// Default font size in layout units.
pub(super) const DEFAULT_FONT_SIZE: f32 = 16.0;

// unit conversion
/// Meters in one inch — the SI definition of the inch.
pub(super) const METERS_PER_INCH: f32 = 0.0254;

/// Minimum `meters_per_unit` for `Unit::Custom`, equal to `Unit::Points`.
///
/// Units smaller than a typographic point would cause font sizes to shrink
/// below 1.0 when converted to points for the layout engine, hitting parley's
/// integer quantization and producing incorrect baselines.
pub(super) const MIN_CUSTOM_METERS_PER_UNIT: f32 = METERS_PER_INCH / POINTS_PER_INCH;

/// Logical pixels per inch at the standard CSS / web typography resolution.
///
/// This is the conversion factor between `Unit::Pixels` and `Unit::Points`:
/// 1 point = 1/72 inch, 1 pixel = 1/[`PIXELS_PER_INCH`] inch, so
/// 1 point = [`PIXELS_PER_INCH`] / 72 pixels (≈ 1.333 at 96 DPI). This makes
/// `Pt(12)` render at 16 logical pixels, matching CSS/Word/etc. conventions.
///
/// Physical pixels on a high-DPI display are still 1:1 with logical pixels
/// via the window's `scale_factor`; we operate in logical pixels throughout.
pub(super) const PIXELS_PER_INCH: f32 = 96.0;

/// Typographic points in one inch (1 point = 1/72 inch).
pub(super) const POINTS_PER_INCH: f32 = 72.0;
