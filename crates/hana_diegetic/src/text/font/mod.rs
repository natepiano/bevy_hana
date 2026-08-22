//! Font-level typographic metrics, parsed from font tables via `ttf_parser`.
//!
//! [`Font`] pre-parses raw design-unit metrics at creation time. Call
//! [`Font::metrics`] to get scaled [`FontMetrics`] at any font size —
//! pure arithmetic, no re-parsing.

mod constants;
mod loader;
mod measurer;
mod registry;

use std::sync::Arc;

use bevy::asset::Asset;
use bevy::reflect::TypePath;
pub(crate) use constants::DEFAULT_FAMILY;
use constants::MONOSPACE_ADVANCE_SAMPLE;
pub(super) use loader::FontLoader;
pub use measurer::DiegeticTextMeasurer;
pub use measurer::create_parley_measurer;
pub use registry::FontId;
pub use registry::FontLoadFailed;
pub use registry::FontRegistered;
pub use registry::FontRegistry;
pub use registry::FontSource;
use ttf_parser::Face;
use ttf_parser::GlyphId;

use crate::layout::Pt;
use crate::layout::Unit;

/// Pre-parsed font with design-unit metrics.
///
/// Created via [`Font::from_bytes`]. All raw values are in the font's
/// design units (`units_per_em`). Call [`Font::metrics`] to get values
/// scaled to a specific font size.
///
/// Also a Bevy [`Asset`] — load `.ttf`/`.otf` files via `AssetServer`:
///
/// ```ignore
/// let handle: Handle<Font> = asset_server.load("fonts/MyFont.ttf");
/// ```
///
/// When the asset loads, the plugin automatically registers it with
/// [`FontRegistry`](crate::FontRegistry) and fires a
/// [`FontRegistered`](crate::FontRegistered) event.
#[derive(Asset, TypePath)]
pub struct Font {
    name:                    String,
    units_per_em:            u16,
    monospace_advance:       MonospaceAdvance,
    raw_ascent:              i16,
    raw_descent:             i16,
    raw_line_gap:            i16,
    raw_cap_height:          i16,
    raw_x_height:            i16,
    raw_italic_angle:        f32,
    raw_underline_position:  Option<i16>,
    raw_underline_thickness: Option<i16>,
    raw_strikeout_position:  Option<i16>,
    raw_strikeout_thickness: Option<i16>,
    /// Raw font bytes, retained for slug curve extraction and per-glyph queries.
    data:                    Arc<[u8]>,
}

/// Why [`Font::nearest_integral_advance_size`] could not resolve a point size.
#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
pub enum IntegralAdvanceSizeError {
    /// The font is not fixed-pitch, so it has no single glyph advance to align.
    #[error("integral advance sizing requires a monospaced font")]
    ProportionalFont,
    /// The font is marked fixed-pitch but provides no positive horizontal advance.
    #[error("monospaced font has no positive horizontal advance")]
    MissingAdvance,
    /// The requested point size is NaN or infinite.
    #[error("requested point size must be finite, got {requested}")]
    NonFiniteRequest {
        /// The rejected point size.
        requested: f32,
    },
    /// The requested point size is zero or negative.
    #[error("requested point size must be positive, got {requested}")]
    NonPositiveRequest {
        /// The rejected point size.
        requested: f32,
    },
}

/// Fixed-pitch state parsed from a font face.
#[derive(Clone, Copy, Debug)]
enum MonospaceAdvance {
    /// The face declares proportional spacing.
    Proportional,
    /// The face declares fixed spacing but has no positive sampled advance.
    Missing,
    /// Shared horizontal advance in font design units.
    DesignUnits(u16),
}

/// Font-level typographic metrics, scaled to a specific font size.
///
/// Returned by [`Font::metrics`]. All distance values are in layout units,
/// scaled from the font's design units by `font_size / units_per_em`.
///
/// Vertical distances are positive in both directions from the baseline:
/// - `ascent` extends **above** the baseline.
/// - `descent` extends **below** the baseline.
pub struct FontMetrics {
    /// Distance from the baseline to the ascender line. This is the font's
    /// full ascender — it includes room for accented characters like `Â` and
    /// `É`, so it is always >= [`cap_height`](Self::cap_height).
    pub ascent:              f32,
    /// Distance from the baseline to the descender line (positive = below
    /// baseline). Covers the lowest descenders like `p`, `g`, `y`.
    pub descent:             f32,
    /// Font-recommended inter-line spacing, also called "leading" in
    /// traditional typography. In parley's half-leading model this value
    /// is split in half and absorbed into the `top` and `bottom` of
    /// each line box — see `LineMetricsSnapshot`.
    pub line_gap:            f32,
    /// Total line height: `ascent + descent + line_gap`.
    pub line_height:         f32,
    /// Height of lowercase letters like `x` (baseline to mean line).
    /// Also called the "mean line" in some references.
    pub x_height:            f32,
    /// Height of uppercase letters like `H` (baseline to cap line).
    /// Always <= [`ascent`](Self::ascent) because ascent includes room
    /// for diacritics above capitals.
    pub cap_height:          f32,
    /// Italic angle in degrees from vertical. `0.0` for upright fonts.
    pub italic_angle:        f32,
    /// Distance below the baseline for underline placement. `None` if the
    /// font's post table does not specify underline metrics — there is no
    /// meaningful fallback.
    pub underline_position:  Option<f32>,
    /// Underline stroke thickness. `None` if the font's post table does not
    /// specify underline metrics.
    pub underline_thickness: Option<f32>,
    /// Distance above the baseline for strikeout placement. `None` if the
    /// font's OS/2 table does not specify strikeout metrics — there is no
    /// meaningful fallback.
    pub strikeout_position:  Option<f32>,
    /// Strikeout stroke thickness. `None` if the font's OS/2 table does not
    /// specify strikeout metrics.
    pub strikeout_thickness: Option<f32>,
    /// The font size these metrics were computed for.
    pub font_size:           f32,
    /// Number of design units per em in the original font.
    pub units_per_em:        u16,
}

/// Bounding rectangle for a single glyph, in scaled layout units.
#[cfg(feature = "typography_overlay")]
pub struct GlyphBounds {
    /// Left edge of the glyph bounding box.
    pub min_x: f32,
    /// Bottom edge of the glyph bounding box.
    pub min_y: f32,
    /// Right edge of the glyph bounding box.
    pub max_x: f32,
    /// Top edge of the glyph bounding box.
    pub max_y: f32,
}

/// Per-glyph typographic metrics, scaled to a specific font size.
///
/// Computed on the fly by [`Font::glyph_metrics`] — never stored
/// persistently. Only available when the `typography_overlay` feature
/// is enabled.
#[cfg(feature = "typography_overlay")]
pub struct GlyphTypographyMetrics {
    /// Horizontal advance width (Apple's "Advancement").
    pub advance_width: f32,
    /// Glyph bounding rectangle.
    pub bounds:        GlyphBounds,
    /// Left side bearing — horizontal distance from the origin to the
    /// left edge of the glyph bounding box.
    pub bearing_x:     f32,
    /// Top side bearing — vertical distance from the baseline to the
    /// top edge of the glyph bounding box.
    pub bearing_y:     f32,
}

impl Font {
    /// Parses font-level metrics from raw TTF/OTF bytes.
    ///
    /// Reads the OS/2, hhea, and post tables to extract ascent, descent,
    /// line gap, cap height, x-height, italic angle, and underline/strikeout
    /// metrics. When `cap_height` or `x_height` are not in the OS/2 table,
    /// they are derived from the bounding box of the `H` or `x` glyph.
    ///
    /// Returns `None` if the font data cannot be parsed.
    #[must_use]
    pub fn from_bytes(name: &str, data: &[u8]) -> Option<Self> {
        let face = Face::parse(data, 0).ok()?;
        let units_per_em = face.units_per_em();
        let monospace_advance = monospace_advance(&face);

        let raw_ascent = face.ascender();
        // ttf-parser returns descender as negative; we store the absolute value
        // so `descent` is always positive (distance below baseline).
        let raw_descent = face.descender().abs();
        let raw_line_gap = face.line_gap();

        // Cap height: prefer OS/2 table, fall back to 'H' glyph bbox.
        let raw_cap_height = face
            .capital_height()
            .unwrap_or_else(|| glyph_top(&face, 'H').unwrap_or(raw_ascent));

        // X-height: prefer OS/2 table, fall back to 'x' glyph bbox.
        let raw_x_height = face
            .x_height()
            .unwrap_or_else(|| glyph_top(&face, 'x').unwrap_or(raw_ascent / 2));

        let raw_italic_angle = face.italic_angle();

        let (raw_underline_position, raw_underline_thickness) =
            face.underline_metrics().map_or((None, None), |m| {
                (Some(m.position.abs()), Some(m.thickness))
            });

        let (raw_strikeout_position, raw_strikeout_thickness) = face
            .strikeout_metrics()
            .map_or((None, None), |m| (Some(m.position), Some(m.thickness)));

        Some(Self {
            name: (*name).to_string(),
            units_per_em,
            monospace_advance,
            raw_ascent,
            raw_descent,
            raw_line_gap,
            raw_cap_height,
            raw_x_height,
            raw_italic_angle,
            raw_underline_position,
            raw_underline_thickness,
            raw_strikeout_position,
            raw_strikeout_thickness,
            data: Arc::from(data),
        })
    }

    /// Returns the font family name.
    #[must_use]
    pub fn name(&self) -> &str { &self.name }

    /// Returns the raw TTF/OTF font bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] { &self.data }

    /// Returns font-level metrics scaled to `size` layout units.
    ///
    /// Pure arithmetic — no parsing, no allocation. The raw design-unit
    /// values are multiplied by `size / units_per_em`.
    #[must_use]
    pub fn metrics(&self, size: f32) -> FontMetrics {
        let scale = size / f32::from(self.units_per_em);

        let ascent = f32::from(self.raw_ascent) * scale;
        let descent = f32::from(self.raw_descent) * scale;
        let line_gap = f32::from(self.raw_line_gap) * scale;

        FontMetrics {
            ascent,
            descent,
            line_gap,
            line_height: ascent + descent + line_gap,
            x_height: f32::from(self.raw_x_height) * scale,
            cap_height: f32::from(self.raw_cap_height) * scale,
            italic_angle: self.raw_italic_angle,
            underline_position: self.raw_underline_position.map(|v| f32::from(v) * scale),
            underline_thickness: self.raw_underline_thickness.map(|v| f32::from(v) * scale),
            strikeout_position: self.raw_strikeout_position.map(|v| f32::from(v) * scale),
            strikeout_thickness: self.raw_strikeout_thickness.map(|v| f32::from(v) * scale),
            font_size: size,
            units_per_em: self.units_per_em,
        }
    }

    /// Returns the nearest point size whose fixed-pitch glyph advance is an
    /// integer number of logical screen pixels.
    ///
    /// Repeated glyphs then retain the same horizontal pixel phase instead of
    /// moving through different fractional positions. The first glyph's origin
    /// remains under the caller's control.
    /// Using the returned size improves the on-screen appearance of monospace
    /// text by keeping glyph-edge coverage consistent across a run.
    ///
    /// The calculation uses the standard screen conversion of 96 logical
    /// pixels per inch and 72 typographic points per inch. It assumes no extra
    /// letter spacing; integral-pixel letter spacing preserves the guarantee.
    /// Ties resolve to the larger point size.
    ///
    /// # Errors
    ///
    /// Returns [`IntegralAdvanceSizeError::ProportionalFont`] when the font is
    /// not marked as monospaced, [`IntegralAdvanceSizeError::MissingAdvance`]
    /// when it has no positive fixed advance, or the corresponding request
    /// error when `requested` is non-finite or non-positive.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let font = registry.font(FontId::MONOSPACE)?;
    /// let size = font.nearest_integral_advance_size(Pt(11.0))?;
    /// let style = TextStyle::new(size);
    /// ```
    pub fn nearest_integral_advance_size(
        &self,
        requested: Pt,
    ) -> Result<Pt, IntegralAdvanceSizeError> {
        if !requested.0.is_finite() {
            return Err(IntegralAdvanceSizeError::NonFiniteRequest {
                requested: requested.0,
            });
        }
        if requested.0 <= 0.0 {
            return Err(IntegralAdvanceSizeError::NonPositiveRequest {
                requested: requested.0,
            });
        }

        let raw_advance = match self.monospace_advance {
            MonospaceAdvance::Proportional => {
                return Err(IntegralAdvanceSizeError::ProportionalFont);
            },
            MonospaceAdvance::Missing => {
                return Err(IntegralAdvanceSizeError::MissingAdvance);
            },
            MonospaceAdvance::DesignUnits(advance) => f32::from(advance),
        };

        let units_per_em = f32::from(self.units_per_em);
        let points_per_pixel = Unit::Pixels.to_points();
        let requested_advance_pixels = requested.0 * raw_advance / units_per_em / points_per_pixel;
        let integral_advance_pixels = requested_advance_pixels.round().max(1.0);
        let point_size = integral_advance_pixels * points_per_pixel * units_per_em / raw_advance;

        Ok(Pt(point_size))
    }

    /// Returns per-glyph typographic metrics for `ch` at `size`.
    ///
    /// Parses the glyph on demand from stored font data. Returns `None`
    /// if the character has no glyph in this font.
    #[cfg(feature = "typography_overlay")]
    #[must_use]
    pub fn glyph_metrics(&self, ch: char, size: f32) -> Option<GlyphTypographyMetrics> {
        let face = Face::parse(&self.data, 0).ok()?;
        let glyph_id = face.glyph_index(ch)?;
        let ink = glyph_ink_extents(&face, glyph_id.0)?;
        let scale = size / ink.units_per_em;

        let advance_width = face
            .glyph_hor_advance(glyph_id)
            .map_or(0.0, |a| f32::from(a) * scale);

        let bounds = GlyphBounds {
            min_x: ink.min_x * scale,
            min_y: ink.min_y * scale,
            max_x: ink.max_x * scale,
            max_y: ink.max_y * scale,
        };

        let bearing_x = bounds.min_x;
        let bearing_y = bounds.max_y;

        Some(GlyphTypographyMetrics {
            advance_width,
            bounds,
            bearing_x,
            bearing_y,
        })
    }

    /// Returns per-glyph typographic metrics by glyph ID at `size`.
    ///
    /// Like [`glyph_metrics`](Self::glyph_metrics) but takes a glyph index
    /// directly (as stored in `ShapedGlyph`)
    /// rather than a character.
    #[cfg(feature = "typography_overlay")]
    #[must_use]
    pub fn glyph_metrics_by_id(&self, glyph_id: u16, size: f32) -> Option<GlyphTypographyMetrics> {
        let face = Face::parse(&self.data, 0).ok()?;
        let ink = glyph_ink_extents(&face, glyph_id)?;
        let scale = size / ink.units_per_em;

        let advance_width = face
            .glyph_hor_advance(GlyphId(glyph_id))
            .map_or(0.0, |a| f32::from(a) * scale);

        let bounds = GlyphBounds {
            min_x: ink.min_x * scale,
            min_y: ink.min_y * scale,
            max_x: ink.max_x * scale,
            max_y: ink.max_y * scale,
        };

        let bearing_x = bounds.min_x;
        let bearing_y = bounds.max_y;

        Some(GlyphTypographyMetrics {
            advance_width,
            bounds,
            bearing_x,
            bearing_y,
        })
    }
}

/// A glyph's ink bounding box in font design units, paired with the font's
/// units-per-em so a caller can scale it to a font size.
///
/// Read straight from the glyph outline, so it is the glyph's true drawn extent
/// — which, for some fonts, reaches past the declared ascent/descent line
/// metrics (a CJK font's Latin descenders, for one). Sizing a box to the line
/// box alone clips that ink; sizing it to these extents does not.
pub(crate) struct GlyphInkExtents {
    /// Left edge in design units.
    pub min_x:        f32,
    /// Bottom edge in design units — negative below the baseline.
    pub min_y:        f32,
    /// Right edge in design units.
    pub max_x:        f32,
    /// Top edge in design units.
    pub max_y:        f32,
    /// Design units per em, for scaling to a font size.
    pub units_per_em: f32,
}

/// Reads a glyph's ink bounding box from its outline, or `None` when the glyph
/// has no outline (a space, say) or the id is absent from the face.
pub(crate) fn glyph_ink_extents(face: &Face<'_>, glyph_id: u16) -> Option<GlyphInkExtents> {
    let bbox = face.glyph_bounding_box(GlyphId(glyph_id))?;
    Some(GlyphInkExtents {
        min_x:        f32::from(bbox.x_min),
        min_y:        f32::from(bbox.y_min),
        max_x:        f32::from(bbox.x_max),
        max_y:        f32::from(bbox.y_max),
        units_per_em: f32::from(face.units_per_em()),
    })
}

/// Returns the top of a glyph's bounding box in design units, or `None`.
fn glyph_top(face: &Face<'_>, ch: char) -> Option<i16> {
    let glyph_id = face.glyph_index(ch)?;
    face.glyph_bounding_box(glyph_id).map(|r| r.y_max)
}

/// Returns the shared fixed-pitch advance in design units.
fn monospace_advance(face: &Face<'_>) -> MonospaceAdvance {
    if !face.is_monospaced() {
        return MonospaceAdvance::Proportional;
    }

    face.glyph_index(MONOSPACE_ADVANCE_SAMPLE)
        .and_then(|glyph_id| face.glyph_hor_advance(glyph_id))
        .filter(|advance| *advance > 0)
        .map_or(MonospaceAdvance::Missing, MonospaceAdvance::DesignUnits)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests use unwrap for clearer failure messages"
)]
mod tests {
    use super::Font;
    use super::IntegralAdvanceSizeError;
    use crate::Pt;

    const JETBRAINS_MONO_DATA: &[u8] =
        include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");
    const NOTO_SANS_DATA: &[u8] = include_bytes!("../../../assets/fonts/NotoSans-Regular.ttf");

    #[test]
    fn nearest_integral_advance_selects_eleven_and_one_quarter_points() {
        let font = Font::from_bytes("JetBrains Mono", JETBRAINS_MONO_DATA).unwrap();
        let resolved = font.nearest_integral_advance_size(Pt(11.0)).unwrap();

        assert!((resolved.0 - 11.25).abs() < f32::EPSILON);
    }

    #[test]
    fn nearest_integral_advance_preserves_an_integral_size() {
        let font = Font::from_bytes("JetBrains Mono", JETBRAINS_MONO_DATA).unwrap();
        let resolved = font.nearest_integral_advance_size(Pt(10.0)).unwrap();

        assert!((resolved.0 - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn nearest_integral_advance_rejects_proportional_fonts() {
        let font = Font::from_bytes("Noto Sans", NOTO_SANS_DATA).unwrap();

        assert!(matches!(
            font.nearest_integral_advance_size(Pt(11.0)),
            Err(IntegralAdvanceSizeError::ProportionalFont)
        ));
    }

    #[test]
    fn nearest_integral_advance_rejects_invalid_sizes() {
        let font = Font::from_bytes("JetBrains Mono", JETBRAINS_MONO_DATA).unwrap();

        assert!(matches!(
            font.nearest_integral_advance_size(Pt(0.0)),
            Err(IntegralAdvanceSizeError::NonPositiveRequest { requested: 0.0 })
        ));
        assert!(matches!(
            font.nearest_integral_advance_size(Pt(f32::NAN)),
            Err(IntegralAdvanceSizeError::NonFiniteRequest { requested })
                if requested.is_nan()
        ));
    }
}
