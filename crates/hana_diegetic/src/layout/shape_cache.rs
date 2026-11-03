//! Shared text-shaping cache keyed by `(text, TextMeasure)`.
//!
//! Stores two parallel maps:
//! - **measurements** — `TextDimensions` used by the layout engine's `MeasureTextFn`. This is the
//!   only half the headless layout path needs.
//! - **entries** — glyph runs from text shaping, reused by the renderer so parley shaping is not
//!   repeated after measurement has already happened.
//!
//! The cache is defined in `layout/` because its key and value types
//! (`TextMeasure`, `TextDimensions`) are layout-domain types and headless
//! panel layout depends on the measurement half. The renderer populates both
//! halves whenever it runs text shaping, so measurement lookups hit when
//! shaping has already run.

use std::collections::HashMap;
use std::hash::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::ops::Range;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use bevy::prelude::Resource;
use hana_kana::ToI32;
use hana_kana::ToU32;

use super::font_features::FontFeatures;
use super::text_props::FontSlant;
use super::text_props::TextDimensions;
use super::text_props::TextMeasure;

/// Resolved font face used by a shaped glyph.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ResolvedFontFace {
    /// Requested `FontId` from the text style.
    pub requested_font_id: u16,
    /// Parley/fontique blob identity for the actual face used by shaping.
    pub blob_id:           u64,
    /// Face index inside a font collection.
    pub collection_index:  u32,
}

/// A single shaped glyph from parley: resolved font, glyph ID, and position
/// relative to the text origin.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedGlyph {
    /// Resolved font face used by this glyph.
    pub font_face: ResolvedFontFace,
    /// Glyph index within the font.
    pub id:        u16,
    /// X position relative to the text origin (accumulated advance + fine
    /// offset).
    pub x:         f32,
    /// Y position relative to the text origin (baseline-relative).
    pub y:         f32,
    /// Baseline of the line this glyph belongs to.
    pub baseline:  f32,
    /// Horizontal advance of this glyph, in the same units as `x`.
    pub advance:   f32,
}

/// Parley's per-line metrics, recorded during text shaping.
///
/// All values are in layout units (Y-down coordinate system).
#[derive(Clone, Copy, Debug)]
pub struct ShapedLineMetrics {
    /// Typographic ascent for this line.
    pub ascent:   f32,
    /// Typographic descent for this line.
    pub descent:  f32,
    /// Offset to the baseline from the top of the layout.
    pub baseline: f32,
    /// Top of the line box (parley `block_min_coord`).
    pub top:      f32,
    /// Bottom of the line box (parley `block_max_coord`).
    pub bottom:   f32,
}

/// Cached shaping result for a text string at a specific font configuration.
#[derive(Clone, Debug)]
pub struct ShapedTextRun {
    /// The shaped glyphs in order.
    pub glyphs:       Vec<ShapedGlyph>,
    /// Per-line metrics from parley, captured during shaping.
    pub line_metrics: Vec<ShapedLineMetrics>,
}

/// Cache key: hash of the text string + the full `TextMeasure` identity.
#[derive(Clone, Eq, PartialEq, Hash)]
struct ShapedCacheKey {
    text_hash:                u64,
    font_id:                  u16,
    /// Size quantized to avoid floating-point hash issues (size * 100 as u32).
    size_quantized:           u32,
    weight_quantized:         u32,
    slant:                    u8,
    line_height_quantized:    u32,
    letter_spacing_quantized: i32,
    word_spacing_quantized:   i32,
    font_features:            FontFeatures,
}

impl ShapedCacheKey {
    fn new(text: &str, m: &TextMeasure) -> Self {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        Self {
            text_hash:                hasher.finish(),
            font_id:                  m.font_id,
            size_quantized:           (m.size * 100.0).to_u32(),
            weight_quantized:         (m.weight.0 * 10.0).to_u32(),
            slant:                    match m.slant {
                FontSlant::Normal => 0,
                FontSlant::Italic => 1,
                FontSlant::Oblique => 2,
            },
            line_height_quantized:    (m.line_height * 100.0).to_u32(),
            letter_spacing_quantized: (m.letter_spacing * 100.0).to_i32(),
            word_spacing_quantized:   (m.word_spacing * 100.0).to_i32(),
            font_features:            m.font_features,
        }
    }
}

/// Caches text-shaping output — glyph runs and measurements — to avoid
/// redundant parley shaping.
///
/// The two maps are stored behind a shared `Arc<Mutex<…>>`, so the cache is a cheap
/// refcount bump to clone and every clone reads and writes the same maps. The
/// layout engine's `MeasureTextFn` (measurement half) clones the handle into its
/// `'static` measure closure; the renderer's text shaper (run + measurement
/// halves) holds it as a `Res`. All methods take `&self` and lock internally, so
/// an insert through any handle is visible to every other handle, so the layout
/// pass neither copies the maps each frame nor throws away the measurements it
/// computes on a miss.
#[derive(Resource, Clone, Default)]
pub struct ShapedTextCache {
    inner: Arc<Mutex<ShapedTextCacheMaps>>,
}

/// The cache's two maps — glyph runs and measurements — guarded together by one
/// mutex so a single lock covers both.
#[derive(Default)]
struct ShapedTextCacheMaps {
    entries:      HashMap<ShapedCacheKey, ShapedTextRun>,
    measurements: HashMap<ShapedCacheKey, TextDimensions>,
    rich_entries: HashMap<RichShapedCacheKey, Arc<ShapedRichParagraph>>,
    rich_shapes:  usize,
}

impl ShapedTextCache {
    /// Returns cached measurement dimensions for the given text + config,
    /// or `None` if not yet cached.
    #[must_use]
    pub fn get_measurement(&self, text: &str, measure: &TextMeasure) -> Option<TextDimensions> {
        let key = ShapedCacheKey::new(text, measure);
        let maps = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        maps.measurements.get(&key).copied()
    }

    /// Returns a clone of the cached glyph run for the given text + config, or
    /// `None` if not yet cached. Returns an owned value because the run is
    /// stored behind the cache's mutex and cannot be borrowed past the lock.
    #[must_use]
    pub fn get_shaped(&self, text: &str, measure: &TextMeasure) -> Option<ShapedTextRun> {
        let key = ShapedCacheKey::new(text, measure);
        let maps = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        maps.entries.get(&key).cloned()
    }

    /// Inserts a measurement result into the cache.
    pub fn insert_measurement(&self, text: &str, measure: &TextMeasure, dims: TextDimensions) {
        let key = ShapedCacheKey::new(text, measure);
        let mut maps = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        maps.measurements.insert(key, dims);
    }

    /// Inserts a glyph run alongside its measurement. Used by the renderer
    /// after parley shaping so subsequent layout-engine lookups hit.
    pub fn insert_shaped(
        &self,
        text: &str,
        measure: &TextMeasure,
        run: ShapedTextRun,
        dims: TextDimensions,
    ) {
        let key = ShapedCacheKey::new(text, measure);
        let mut maps = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        maps.measurements.insert(key.clone(), dims);
        maps.entries.insert(key, run);
    }
}

/// Geometry identity in resolved layout units, independent of colour-run boundaries.
/// Font scale enters each resolved measurement, allowing pre-scaled render commands
/// to reuse the exact paragraph measured by layout.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct RichShapedCacheKey {
    text:   String,
    base:   RichMeasureKey,
    ranges: Vec<(Range<usize>, RichMeasureKey)>,
}
impl RichShapedCacheKey {
    pub(crate) fn new(paragraph: &super::RichParagraph, scale: f32) -> Self {
        Self {
            text:   paragraph.text.clone(),
            base:   RichMeasureKey::new(&paragraph.base.as_measure().scaled(scale)),
            ranges: paragraph
                .geometry_runs()
                .into_iter()
                .map(|(range, measure)| (range, RichMeasureKey::new(&measure.scaled(scale))))
                .collect(),
        }
    }
}

/// One visual cluster, retaining its source range for paint-only regrouping.
#[derive(Clone, Debug)]
pub(crate) struct RichGlyphCluster {
    pub range:      Range<usize>,
    pub glyphs:     Vec<ShapedGlyph>,
    pub line_index: usize,
}

/// One cached paragraph's geometry, shared by measurement and rendering.
#[derive(Debug)]
pub(crate) struct ShapedRichParagraph {
    pub clusters:     Vec<RichGlyphCluster>,
    pub line_metrics: Vec<ShapedLineMetrics>,
    pub line_widths:  Vec<f32>,
    pub dimensions:   TextDimensions,
}

/// A rich cache lookup names whether paragraph shaping remains necessary.
pub(crate) enum RichShapeCacheLookup {
    Cached(Arc<ShapedRichParagraph>),
    NeedsShaping,
}
impl ShapedTextCache {
    /// Cumulative completed rich paragraph shapes, for cache-reuse diagnostics.
    #[doc(hidden)]
    #[must_use]
    pub fn rich_shape_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .rich_shapes
    }

    pub(crate) fn rich_lookup(
        &self,
        paragraph: &super::RichParagraph,
        scale: f32,
    ) -> RichShapeCacheLookup {
        let key = RichShapedCacheKey::new(paragraph, scale);
        let maps = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        match maps.rich_entries.get(&key) {
            Some(shaped) => RichShapeCacheLookup::Cached(Arc::clone(shaped)),
            None => RichShapeCacheLookup::NeedsShaping,
        }
    }
    pub(crate) fn insert_rich(
        &self,
        paragraph: &super::RichParagraph,
        scale: f32,
        shaped: Arc<ShapedRichParagraph>,
    ) {
        let key = RichShapedCacheKey::new(paragraph, scale);
        let mut maps = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        maps.rich_entries.insert(key, shaped);
        maps.rich_shapes += 1;
    }
}

/// Exact rich measurement identity; tiny spacing changes must remain measurable.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct RichMeasureKey {
    font_id:        u16,
    size:           u32,
    weight:         u32,
    slant:          u8,
    line_height:    u32,
    letter_spacing: u32,
    word_spacing:   u32,
    features:       FontFeatures,
}
impl RichMeasureKey {
    const fn new(measure: &TextMeasure) -> Self {
        Self {
            font_id:        measure.font_id,
            size:           measure.size.to_bits(),
            weight:         measure.weight.0.to_bits(),
            slant:          match measure.slant {
                FontSlant::Normal => 0,
                FontSlant::Italic => 1,
                FontSlant::Oblique => 2,
            },
            line_height:    measure.line_height.to_bits(),
            letter_spacing: measure.letter_spacing.to_bits(),
            word_spacing:   measure.word_spacing.to_bits(),
            features:       measure.font_features,
        }
    }
}

#[cfg(test)]
mod rich_cache_tests {
    use bevy::color::Color;

    use super::*;
    use crate::RichTextBuilder;
    use crate::SpanStyle;
    use crate::TextSizing;
    use crate::TextStyle;
    use crate::TextWrap;

    fn paragraph(colors: [Color; 2], spacing: f32) -> crate::RichParagraph {
        let mut builder =
            RichTextBuilder::new(TextStyle::new(12.0), TextSizing::wrap(TextWrap::None));
        builder
            .span((
                "left ",
                SpanStyle::new()
                    .with_color(colors[0])
                    .with_word_spacing(spacing),
            ))
            .span((
                "right",
                SpanStyle::new()
                    .with_color(colors[1])
                    .with_word_spacing(spacing),
            ));
        builder.finish()
    }

    #[test]
    fn rich_cache_survives_merged_paint_boundaries() {
        let original = paragraph([Color::WHITE, Color::BLACK], 0.0);
        let merged = paragraph([Color::BLACK; 2], 0.0);
        let cache = ShapedTextCache::default();
        let geometry = Arc::new(ShapedRichParagraph {
            clusters:     Vec::new(),
            line_metrics: Vec::new(),
            line_widths:  Vec::new(),
            dimensions:   TextDimensions::default(),
        });
        cache.insert_rich(&original, 1.0, Arc::clone(&geometry));
        assert!(matches!(cache.rich_lookup(&merged, 1.0),
            RichShapeCacheLookup::Cached(reused) if Arc::ptr_eq(&geometry, &reused)));
        assert_eq!(
            original.classify_change(&merged),
            crate::LayoutTreeChange::VisualOnly
        );
    }

    #[test]
    fn rich_cache_reuses_unit_converted_measurement_for_rendering() {
        let source = paragraph([Color::WHITE, Color::BLACK], 1.0);
        let rendered = source.scaled(2.0);
        assert_eq!(
            RichShapedCacheKey::new(&source, 2.0),
            RichShapedCacheKey::new(&rendered, 1.0)
        );
    }

    #[test]
    fn rich_cache_distinguishes_subpixel_spacing_and_scale() {
        let original = paragraph([Color::WHITE; 2], 0.001);
        let changed = paragraph([Color::WHITE; 2], 0.002);
        let cache = ShapedTextCache::default();
        cache.insert_rich(
            &original,
            1.0,
            Arc::new(ShapedRichParagraph {
                clusters:     Vec::new(),
                line_metrics: Vec::new(),
                line_widths:  Vec::new(),
                dimensions:   TextDimensions::default(),
            }),
        );
        assert!(matches!(
            cache.rich_lookup(&changed, 1.0),
            RichShapeCacheLookup::NeedsShaping
        ));
        assert!(matches!(
            cache.rich_lookup(&original, 2.0),
            RichShapeCacheLookup::NeedsShaping
        ));
    }
}
