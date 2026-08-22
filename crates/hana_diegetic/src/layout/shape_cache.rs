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
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use bevy::prelude::Resource;
use bevy_kana::ToI32;
use bevy_kana::ToU32;

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
#[derive(Clone, Debug)]
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

/// Snapshot of parley's per-line metrics, captured during text shaping.
///
/// All values are in layout units (Y-down coordinate system).
#[derive(Clone, Copy, Debug)]
pub struct LineMetricsSnapshot {
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
    pub line_metrics: Vec<LineMetricsSnapshot>,
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
/// pass neither copies the maps each frame nor discards the misses it computes.
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
