//! Test helpers that build text runs through the production positioned-glyph
//! path, so unit tests exercise the same entry point the renderer uses.
#![allow(
    clippy::expect_used,
    reason = "fixtures should fail loudly when bundled font data is missing"
)]

use bevy::math::Vec2;
use ttf_parser::Face;
use ttf_parser::GlyphId;

use crate::layout::ResolvedFontFace;
use crate::layout::ShapedGlyph;
use crate::render::DEFAULT_BAND_COUNT;
use crate::text::Font;
use crate::text::slug::glyph::OutlineError;
use crate::text::slug::runtime::BuiltTextRun;
use crate::text::slug::runtime::CachedGlyphOutline;
use crate::text::slug::runtime::FontKey;
use crate::text::slug::runtime::GlyphCache;
use crate::text::slug::runtime::GlyphInstance;
use crate::text::slug::runtime::GlyphKey;
use crate::text::slug::runtime::GlyphOutlineCache;
use crate::text::slug::runtime::PositionedGlyph;
use crate::text::slug::runtime::PreparedTextRun;
use crate::text::slug::runtime::TextRun;

/// Replacement character used as the diagnostic glyph label when packing
/// fixture glyphs by id.
const FIXTURE_DIAGNOSTIC_CHAR: char = '\u{FFFD}';

/// Layout font size used by run fixtures; matched to a 1000-unit em so the
/// per-glyph bounds scale resolves to the placement scale.
pub(super) const FIXTURE_FONT_SIZE: f32 = 1000.0;
/// Placement scale used by run fixtures, mapping design units to world units.
pub(super) const FIXTURE_SCALE: f32 = 0.001;

/// Builds one prepared text run from `text` using the production path: glyph
/// IDs and advances come straight from the font face, mirroring what parley
/// shaping feeds [`GlyphCache::prepare_positioned_run_with_scale`].
pub(in crate::text::slug) fn prepare_fixture_run(
    backend: &mut GlyphCache,
    font_data: &[u8],
    font_key: u64,
    text: &str,
) -> Result<PreparedTextRun, OutlineError> {
    let font = Font::from_bytes("Fixture", font_data).expect("fixture font should parse");
    let glyphs = fixture_shaped_glyphs(font_data, font_key, text);
    let positioned: Vec<PositionedGlyph<'_>> = glyphs
        .iter()
        .map(|glyph| PositionedGlyph {
            glyph,
            font: &font,
            collection_index: 0,
        })
        .collect();
    backend.prepare_positioned_run_with_scale(
        &positioned,
        Vec2::ZERO,
        FIXTURE_FONT_SIZE,
        Vec2::splat(FIXTURE_SCALE),
        DEFAULT_BAND_COUNT,
    )
}

/// Builds a positioned run and its populated glyph cache directly, without a
/// backend, for run-render tests that inspect mesh and storage output.
pub(in crate::text::slug) fn fixture_run_with_cache(
    font_data: &[u8],
    font_key: u64,
    text: &str,
) -> (BuiltTextRun, GlyphOutlineCache) {
    let face = Face::parse(font_data, 0).expect("fixture font should parse");
    let key_seed = FontKey::new(font_key);
    let mut cache = GlyphOutlineCache::default();
    let mut instances = Vec::new();
    let mut origin_x = 0.0_f32;
    for character in text.chars() {
        let glyph_id = face.glyph_index(character).map_or(0, |id| id.0);
        let advance = face
            .glyph_hor_advance(GlyphId(glyph_id))
            .map_or(0.0, f32::from);
        let key = GlyphKey::with_preprocess_version(key_seed, glyph_id, 0);
        let outline = cache
            .get_or_insert_packed_from_face(
                key,
                font_data,
                0,
                FIXTURE_DIAGNOSTIC_CHAR,
                DEFAULT_BAND_COUNT,
            )
            .expect("fixture glyph should pack");
        if let CachedGlyphOutline::Visible(outline) = outline {
            instances.push(GlyphInstance::new_non_uniform(
                key,
                Vec2::new(origin_x * FIXTURE_SCALE, 0.0),
                outline.bounds(),
                Vec2::splat(FIXTURE_SCALE),
            ));
        }
        origin_x += advance;
    }
    (
        BuiltTextRun {
            run: TextRun::new(instances),
        },
        cache,
    )
}

/// Lays out `text` left to right using the font's own glyph indices and
/// advances, producing the [`ShapedGlyph`] values parley would emit.
fn fixture_shaped_glyphs(font_data: &[u8], font_key: u64, text: &str) -> Vec<ShapedGlyph> {
    let face = Face::parse(font_data, 0).expect("fixture font should parse");
    let font_face = ResolvedFontFace {
        requested_font_id: 0,
        blob_id:           font_key,
        collection_index:  0,
    };
    let mut origin_x = 0.0_f32;
    let mut glyphs = Vec::new();
    for character in text.chars() {
        let glyph_id = face.glyph_index(character).map_or(0, |id| id.0);
        let advance = face
            .glyph_hor_advance(GlyphId(glyph_id))
            .map_or(0.0, f32::from);
        glyphs.push(ShapedGlyph {
            font_face,
            id: glyph_id,
            x: origin_x,
            y: 0.0,
            baseline: 0.0,
            advance,
        });
        origin_x += advance;
    }
    glyphs
}
