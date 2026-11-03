use std::collections::HashMap;
use std::collections::hash_map::Entry;

use bevy::math::Vec2;
use hana_kana::ToU32;
use owned_ttf_parser::AsFaceRef;
use owned_ttf_parser::OwnedFace;
use ttf_parser::Face;
#[cfg(test)]
use ttf_parser::GlyphId;

use crate::render::BandRecord;
use crate::render::Bounds;
use crate::render::CurveRecord;
use crate::render::PackedPath;
use crate::render::PackedPathRecord;
use crate::text::slug::glyph;
use crate::text::slug::glyph::Glyph;
use crate::text::slug::glyph::GlyphOutline;
use crate::text::slug::glyph::OutlineError;

/// Stable identity for the resolved font face used by shaping.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct FontKey {
    blob_id:          u64,
    collection_index: u32,
}

impl FontKey {
    /// Creates a font key from a stable caller-owned identifier.
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn new(value: u64) -> Self { Self::for_face(value, 0) }

    /// Distinguishes individual faces within a font collection.
    #[must_use]
    pub(crate) const fn for_face(blob_id: u64, collection_index: u32) -> Self {
        Self {
            blob_id,
            collection_index,
        }
    }
}

/// Cache key for one resolved font glyph.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct GlyphKey {
    font:     FontKey,
    glyph_id: u16,
    packing:  GlyphPackingProfile,
}

impl GlyphKey {
    /// Creates a cache key for one preprocessing version.
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn with_preprocess_version(
        font: FontKey,
        glyph_id: u16,
        preprocess_version: u32,
    ) -> Self {
        Self::for_packing(
            font,
            glyph_id,
            preprocess_version,
            crate::render::DEFAULT_BAND_COUNT,
        )
    }

    /// Identifies an immutable packed result, including its band layout.
    #[must_use]
    pub(crate) const fn for_packing(
        font: FontKey,
        glyph_id: u16,
        preprocess_version: u32,
        band_count: usize,
    ) -> Self {
        Self {
            font,
            glyph_id,
            packing: GlyphPackingProfile {
                preprocess_version,
                band_count: if band_count == 0 { 1 } else { band_count },
            },
        }
    }
}

/// Identity of an extracted outline, independent of GPU packing.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FontGlyphKey {
    font:     FontKey,
    glyph_id: u16,
}

/// Parameters that determine the immutable GPU records for an outline.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct GlyphPackingProfile {
    preprocess_version: u32,
    band_count:         usize,
}

/// One extracted outline and every packed profile requested for it.
#[derive(Debug)]
struct PreparedFontGlyph {
    outline: GlyphOutline,
    packed:  HashMap<GlyphPackingProfile, CachedGlyphOutline>,
}

/// One positioned glyph in a text run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GlyphInstance {
    key:          GlyphKey,
    origin:       Vec2,
    bounds:       Bounds,
    bounds_scale: Vec2,
}

impl GlyphInstance {
    /// Creates a positioned glyph instance with non-uniform scaled bounds.
    #[must_use]
    pub(crate) const fn new_non_uniform(
        key: GlyphKey,
        origin: Vec2,
        bounds: Bounds,
        bounds_scale: Vec2,
    ) -> Self {
        Self {
            key,
            origin,
            bounds,
            bounds_scale,
        }
    }

    /// Key for reusable packed glyph data.
    #[must_use]
    pub(crate) const fn key(self) -> GlyphKey { self.key }

    /// Glyph origin in run layout units.
    #[must_use]
    pub(crate) const fn origin(self) -> Vec2 { self.origin }

    /// Glyph bounds in font design-space units.
    #[must_use]
    pub(crate) const fn bounds(self) -> Bounds { self.bounds }

    /// Scale from glyph design-space bounds to run layout units.
    #[must_use]
    pub(crate) const fn bounds_scale(self) -> Vec2 { self.bounds_scale }
}

/// CPU representation of one positioned text run.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::text::slug) struct TextRun {
    glyphs: Vec<GlyphInstance>,
}

/// Result of positioning and packing one text run.
#[derive(Clone, Debug)]
pub(in crate::text::slug) struct BuiltTextRun {
    /// Per-entity positioned text run.
    pub run: TextRun,
}

impl TextRun {
    /// Creates a text run from already-positioned glyph instances.
    #[must_use]
    pub(in crate::text::slug) const fn new(glyphs: Vec<GlyphInstance>) -> Self { Self { glyphs } }

    /// Ordered glyph instances in run order.
    #[must_use]
    pub(in crate::text::slug) fn glyphs(&self) -> &[GlyphInstance] { &self.glyphs }
}

/// One glyph's cached preparation result: packed curve data for a glyph that
/// draws, or the recorded fact that it has no visible outline (a space or
/// other blank glyph). Caching the invisible answer too means a repeat
/// sighting resolves from one map hit, with no font re-parse either way.
#[derive(Clone, Debug)]
pub(in crate::text::slug) enum CachedGlyphOutline {
    /// Packed curve/band outline for a glyph that draws.
    Visible(PackedPath),
    /// The glyph has no visible outline; runs skip it without atlas work.
    Invisible,
}

/// Cache of reusable packed glyph data, plus the shared glyph atlas every text
/// run indexes into.
///
/// `glyphs` holds each extracted font outline and its immutable packed profiles
/// (or explicit invisibility — see [`CachedGlyphOutline`]). Font metrics share
/// these same entries, so the figure-eight outline is never extracted twice. The atlas fields
/// (`curves` / `bands` / `glyph_records`) are the append-only GPU tables: the
/// first time a glyph is packed, its records are appended here with global
/// offsets and its slot recorded in `record_indices`, so every run that draws
/// the glyph stores that one global index in its mesh instead of copying the
/// glyph's curves per run. `revision` bumps on every append; the GPU upload
/// compares it against the revision it last uploaded and re-uploads only when
/// the two differ. The atlas is append-only — glyphs are never evicted.
#[derive(Debug, Default)]
pub(in crate::text::slug) struct GlyphOutlineCache {
    faces:               HashMap<FontKey, OwnedFace>,
    glyphs:              HashMap<FontGlyphKey, PreparedFontGlyph>,
    record_indices:      HashMap<GlyphKey, u32>,
    curves:              Vec<CurveRecord>,
    bands:               Vec<BandRecord>,
    glyph_records:       Vec<PackedPathRecord>,
    figure_crossbar_mid: HashMap<FontKey, f32>,
    revision:            u32,
}

impl GlyphOutlineCache {
    /// Retains each parsed face for the same lifetime as its packed glyphs.
    fn font_face(&mut self, key: FontKey, data: &[u8]) -> Result<&OwnedFace, OutlineError> {
        match self.faces.entry(key) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let face = OwnedFace::from_vec(data.to_vec(), key.collection_index)
                    .map_err(|_| OutlineError::InvalidFont)?;
                Ok(entry.insert(face))
            },
        }
    }

    /// Design-space em from the face retained by this cache.
    pub(in crate::text::slug) fn font_units_per_em(
        &mut self,
        key: FontKey,
        data: &[u8],
    ) -> Result<f32, OutlineError> {
        Ok(f32::from(
            self.font_face(key, data)?.as_face_ref().units_per_em(),
        ))
    }

    /// Global atlas slot for `key`, if the glyph has been packed.
    #[must_use]
    pub(in crate::text::slug) fn global_index(&self, key: GlyphKey) -> Option<u32> {
        self.record_indices.get(&key).copied()
    }

    /// Shared curve table for every packed glyph.
    #[must_use]
    pub(in crate::text::slug) fn atlas_curves(&self) -> &[CurveRecord] { &self.curves }

    /// Shared band table for every packed glyph.
    #[must_use]
    pub(in crate::text::slug) fn atlas_bands(&self) -> &[BandRecord] { &self.bands }

    /// Shared glyph-record table indexed by each run's mesh.
    #[must_use]
    pub(in crate::text::slug) fn atlas_glyph_records(&self) -> &[PackedPathRecord] {
        &self.glyph_records
    }

    /// Append counter; bumps whenever a new glyph grows the atlas tables.
    #[must_use]
    pub(in crate::text::slug) const fn atlas_revision(&self) -> u32 { self.revision }

    /// Loads, packs, and caches one glyph from a specific collection face,
    /// appending its records to the shared atlas the first time it is seen. A
    /// glyph with no visible outline (a space) caches as
    /// [`CachedGlyphOutline::Invisible`] without touching the atlas.
    pub(in crate::text::slug) fn get_or_insert_packed_from_face(
        &mut self,
        key: GlyphKey,
        font_data: &[u8],
        face_index: u32,
        character: char,
        band_count: usize,
        text_em_size: f32,
    ) -> Result<&CachedGlyphOutline, OutlineError> {
        debug_assert_eq!(key.font.collection_index, face_index);
        debug_assert_eq!(key.packing.band_count, band_count.max(1));
        self.font_face(key.font, font_data)?;
        let face = self.faces[&key.font].as_face_ref();
        let font_glyph_key = FontGlyphKey {
            font:     key.font,
            glyph_id: key.glyph_id,
        };
        let needs_visible_packing = {
            let prepared = prepare_font_glyph(&mut self.glyphs, font_glyph_key, face, character)?;
            matches!(prepared.outline, glyph::GlyphOutline::Visible(_))
                && !prepared.packed.contains_key(&key.packing)
        };
        let figure_crossbar_mid = if needs_visible_packing {
            *self.figure_crossbar_mid.entry(key.font).or_insert_with(|| {
                let fallback =
                    f32::from(face.capital_height().unwrap_or_else(|| face.ascender())) * 0.5;
                let Some(eight) = face.glyph_index('8') else {
                    return fallback;
                };
                let metric_key = FontGlyphKey {
                    font:     key.font,
                    glyph_id: eight.0,
                };
                match prepare_font_glyph(&mut self.glyphs, metric_key, face, '8') {
                    Ok(PreparedFontGlyph {
                        outline: GlyphOutline::Visible(outline),
                        ..
                    }) => figure_crossbar_mid_from_outline(outline, fallback),
                    _ => fallback,
                }
            })
        } else {
            0.0
        };
        let prepared = prepare_font_glyph(&mut self.glyphs, font_glyph_key, face, character)?;
        match prepared.packed.entry(key.packing) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let glyph = match &prepared.outline {
                    GlyphOutline::Invisible => {
                        return Ok(entry.insert(CachedGlyphOutline::Invisible));
                    },
                    GlyphOutline::Visible(glyph) => glyph.clone(),
                };
                let outline = glyph::build_packed_glyph(glyph, key.packing.band_count);

                // First sighting of this glyph: append its packed records to the
                // shared atlas with global offsets and record its slot, so runs
                // index in by one number rather than copying curves per run.
                let record_index = self.glyph_records.len().to_u32();
                let curve_start = self.curves.len().to_u32();
                let band_start = self.bands.len().to_u32();
                let axis_band_count = outline.bands().len().to_u32() / 2;
                self.curves.extend_from_slice(outline.curves());
                self.bands
                    .extend(outline.bands().iter().map(|band| BandRecord {
                        start: band.start + curve_start,
                        ..*band
                    }));
                self.glyph_records.push(
                    PackedPathRecord::new(
                        outline.bounds(),
                        band_start,
                        axis_band_count,
                        band_start + axis_band_count,
                        axis_band_count,
                        0.0,
                    )
                    .with_text_em_size(text_em_size)
                    .with_figure_crossbar_mid(figure_crossbar_mid)
                    .with_has_island(
                        outline
                            .curves()
                            .iter()
                            .any(|curve| curve.island_extent_xy != 0),
                    ),
                );
                self.record_indices.insert(key, record_index);
                self.revision = self.revision.wrapping_add(1);

                Ok(entry.insert(CachedGlyphOutline::Visible(outline)))
            },
        }
    }
}

/// Uses the existing glyph entry for both label geometry and font metrics.
fn prepare_font_glyph<'cache>(
    glyphs: &'cache mut HashMap<FontGlyphKey, PreparedFontGlyph>,
    key: FontGlyphKey,
    face: &Face<'_>,
    character: char,
) -> Result<&'cache mut PreparedFontGlyph, OutlineError> {
    match glyphs.entry(key) {
        Entry::Occupied(entry) => Ok(entry.into_mut()),
        Entry::Vacant(entry) => Ok(entry.insert(PreparedFontGlyph {
            outline: glyph::prepare_glyph_outline(face, key.glyph_id, character)?,
            packed:  HashMap::new(),
        })),
    }
}

/// Figure-eight counter bounds identify the crossbar in this font. Faces
/// without two counters use half their cap height as a stable fallback.
#[cfg(test)]
pub(in crate::text::slug) fn font_figure_crossbar_mid(
    font_data: &[u8],
    face_index: u32,
    em: f32,
) -> f32 {
    let Ok(face) = Face::parse(font_data, face_index) else {
        return em * 0.365;
    };
    figure_crossbar_mid_from_face(&face)
}

#[cfg(test)]
fn figure_crossbar_mid_from_face(face: &Face<'_>) -> f32 {
    let fallback = f32::from(face.capital_height().unwrap_or_else(|| face.ascender())) * 0.5;
    let Some(eight) = face.glyph_index('8') else {
        return fallback;
    };
    let Ok(outline) = glyph::load_glyph_from_face(face, GlyphId(eight.0), '8') else {
        return fallback;
    };
    figure_crossbar_mid_from_outline(&outline, fallback)
}

fn figure_crossbar_mid_from_outline(outline: &Glyph, fallback: f32) -> f32 {
    let mut contours: Vec<(f32, f32, f32)> = outline
        .contours
        .iter()
        .filter_map(|contour| {
            let first = contour.segments.first()?;
            let mut min = first.start;
            let mut max = first.start;
            for segment in &contour.segments {
                for point in [segment.start, segment.control, segment.end] {
                    min = min.min(point);
                    max = max.max(point);
                }
            }
            Some(((max.x - min.x) * (max.y - min.y), min.y, max.y))
        })
        .collect();
    if contours.len() < 3 {
        return fallback;
    }
    contours.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut holes = [contours[1], contours[2]];
    holes.sort_by(|a, b| a.1.total_cmp(&b.1));
    f32::midpoint(holes[0].2, holes[1].1)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "embedded fonts and cache preparation must succeed"
)]
mod tests {
    use hana_kana::ToUsize;

    use super::*;
    use crate::text::slug::glyph::GlyphOutline;

    const MONO: &[u8] = include_bytes!("../../../../assets/fonts/JetBrainsMono-Regular.ttf");
    const SANS: &[u8] = include_bytes!("../../../../assets/fonts/LiberationSans-Regular.ttf");

    #[test]
    fn packed_glyph_keys_separate_faces_and_packing_profiles() {
        let font = FontKey::for_face(11, 0);
        let key = GlyphKey::for_packing(font, 42, 0, 96);
        assert_ne!(
            key,
            GlyphKey::for_packing(FontKey::for_face(11, 1), 42, 0, 96)
        );
        assert_ne!(key, GlyphKey::for_packing(font, 42, 1, 96));
        assert_ne!(key, GlyphKey::for_packing(font, 42, 0, 8));
        assert_eq!(
            GlyphKey::for_packing(font, 42, 0, 0),
            GlyphKey::for_packing(font, 42, 0, 1)
        );
    }

    #[test]
    fn packed_glyphs_and_blank_answers_reuse_the_retained_face() -> Result<(), OutlineError> {
        let face = Face::parse(MONO, 0).expect("mono");
        let font = FontKey::new(11);
        let mut cache = GlyphOutlineCache::default();
        let a = face.glyph_index('A').expect("A").0;
        let space = face.glyph_index(' ').expect("space").0;
        for (id, character, visible) in [(a, 'A', true), (space, ' ', false)] {
            let key = GlyphKey::for_packing(font, id, 0, 96);
            let result = cache
                .get_or_insert_packed_from_face(key, MONO, 0, character, 96, 1000.0)
                .expect("first outline");
            assert_eq!(matches!(result, CachedGlyphOutline::Visible(_)), visible);
            let revision = cache.atlas_revision();
            let index = cache.global_index(key);
            // The retained face and packed answer no longer borrow caller bytes.
            let reused = cache
                .get_or_insert_packed_from_face(
                    key,
                    b"caller bytes no longer available",
                    0,
                    character,
                    96,
                    1000.0,
                )
                .expect("retained face");
            assert_eq!(matches!(reused, CachedGlyphOutline::Visible(_)), visible);
            assert_eq!(cache.atlas_revision(), revision);
            assert_eq!(cache.global_index(key), index);
        }
        assert_eq!(cache.faces.len(), 1);
        assert_eq!(
            cache.glyphs.len(),
            3,
            "A, space and the shared figure-eight metric"
        );
        assert_eq!(cache.glyph_records.len(), 1);
        let key = GlyphKey::for_packing(font, a, 0, 8);
        let CachedGlyphOutline::Visible(packed) = cache
            .get_or_insert_packed_from_face(key, b"already retained", 0, 'A', 8, 1000.0)
            .expect("new packing profile")
        else {
            return Err(OutlineError::MissingGlyphId(a));
        };
        assert_eq!(packed.bands().len(), 16);
        assert_eq!(cache.faces.len(), 1);
        assert_eq!(cache.glyph_records.len(), 2);
        let eight = face.glyph_index('8').expect("eight").0;
        let raw_key = FontGlyphKey {
            font,
            glyph_id: eight,
        };
        let GlyphOutline::Visible(metric_outline) = &cache.glyphs[&raw_key].outline else {
            return Err(OutlineError::MissingGlyphId(eight));
        };
        let contours = metric_outline.contours.as_ptr();
        let key = GlyphKey::for_packing(font, eight, 0, 96);
        cache
            .get_or_insert_packed_from_face(key, b"retained metric outline", 0, '8', 96, 1000.0)
            .expect("pack metric glyph");
        let GlyphOutline::Visible(reused_outline) = &cache.glyphs[&raw_key].outline else {
            return Err(OutlineError::MissingGlyphId(eight));
        };
        assert_eq!(reused_outline.contours.as_ptr(), contours);
        assert_eq!(cache.glyphs.len(), 3);
        assert_eq!(cache.glyph_records.len(), 3);
        Ok(())
    }

    fn collection() -> Vec<u8> {
        let mut bytes = vec![0; 20];
        bytes[..4].copy_from_slice(b"ttcf");
        bytes[4..8].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        bytes[8..12].copy_from_slice(&2_u32.to_be_bytes());
        for (index, font) in [MONO, SANS].into_iter().enumerate() {
            let offset = bytes.len();
            bytes[12 + index * 4..16 + index * 4].copy_from_slice(&(offset.to_u32()).to_be_bytes());
            bytes.extend_from_slice(font);
            let table_count = usize::from(u16::from_be_bytes([font[4], font[5]]));
            for table in 0..table_count {
                let record = offset + 12 + table * 16 + 8;
                let original =
                    u32::from_be_bytes(bytes[record..record + 4].try_into().expect("offset"));
                bytes[record..record + 4]
                    .copy_from_slice(&(original + offset.to_u32()).to_be_bytes());
            }
            while !bytes.len().is_multiple_of(4) {
                bytes.push(0);
            }
        }
        bytes
    }

    #[test]
    fn collection_faces_keep_separate_metrics_and_atlas_records() {
        let bytes = collection();
        let mut cache = GlyphOutlineCache::default();
        for (index, em) in [(0, 1000.0), (1, 2048.0)] {
            let font = FontKey::for_face(11, index);
            assert_eq!(
                cache
                    .font_units_per_em(font, &bytes)
                    .expect("collection face"),
                em
            );
            let key = GlyphKey::for_packing(font, 0, 0, 96);
            assert!(matches!(
                cache
                    .get_or_insert_packed_from_face(key, &bytes, index, '?', 96, em)
                    .expect("collection glyph"),
                CachedGlyphOutline::Visible(_)
            ));
            let record =
                cache.atlas_glyph_records()[cache.global_index(key).expect("slot").to_usize()];
            assert_eq!(record.text_em_size, em);
        }
        assert_eq!(cache.faces.len(), 2);
        assert_eq!(cache.glyph_records.len(), 2);
    }
}
