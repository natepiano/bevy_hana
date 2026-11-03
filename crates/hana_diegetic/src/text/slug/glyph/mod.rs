#[cfg(test)]
mod coverage_probe;
mod outline;

pub(super) use outline::Glyph;
pub(super) use outline::GlyphOutline;
pub(super) use outline::OutlineError;
pub(in crate::text) use outline::font_glyph_id_has_visible_outline;
pub(in crate::text) use outline::load_glyph_by_id_from_face;
#[cfg(test)]
pub(super) use outline::load_glyph_from_face;
pub(super) use outline::prepare_glyph_outline;

use crate::render;
use crate::render::PackedPath;
use crate::render::PathOutline;

/// Text bridge from font-extracted glyphs to renderer-owned analytic paths.
#[must_use]
pub(super) fn build_packed_glyph(glyph: Glyph, band_count: usize) -> PackedPath {
    let path = PathOutline {
        bounds:   glyph.bounds,
        contours: glyph.contours,
    };
    render::build_packed_path(path, band_count)
}
