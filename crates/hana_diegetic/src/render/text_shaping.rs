//! Shared text shaping support used by panel and world text rendering.

use std::borrow::Cow;
use std::sync::Mutex;
use std::sync::PoisonError;

use bevy::prelude::Resource;
use bevy_kana::ToU16;
use parley::Layout;
use parley::LayoutContext;
use parley::RangedBuilder;
use parley::layout::GlyphRun;
use parley::layout::PositionedLayoutItem;
use parley::style::FontFeatures;
use parley::style::FontStyle;
use parley::style::FontWeight;
use parley::style::LineHeight;
use parley::style::StyleProperty;

use crate::layout::FontSlant;
use crate::layout::LineMetricsSnapshot;
use crate::layout::ResolvedFontFace;
use crate::layout::ShapedGlyph;
use crate::layout::ShapedTextCache;
use crate::layout::ShapedTextRun;
use crate::layout::TextDimensions;
use crate::layout::TextStyle;
use crate::text::DEFAULT_FAMILY;
use crate::text::FontId;
use crate::text::FontRegistry;
use crate::text::PositionedGlyph;

/// Reusable parley shaping buffers.
///
/// Avoids reallocating `LayoutContext` and `Layout` on every
/// shaping call. Wrapped in `Mutex` for `Send + Sync`.
#[derive(Resource)]
pub(crate) struct TextShapingContext {
    layout_cx: Mutex<LayoutContext<()>>,
    layout:    Mutex<Layout<()>>,
}

impl Default for TextShapingContext {
    fn default() -> Self {
        Self {
            layout_cx: Mutex::new(parley::LayoutContext::default()),
            layout:    Mutex::new(parley::Layout::new()),
        }
    }
}

/// Timing diagnostics gathered while building text quads.
#[derive(Clone, Debug, Default)]
pub(super) struct TextBuildStats {
    pub texts:            usize,
    pub glyphs:           usize,
    pub ready_glyphs:     usize,
    pub invisible_glyphs: usize,
    pub failed_glyphs:    usize,
    pub emitted_quads:    usize,
    pub shape_ms:         f32,
    pub atlas_ms:         f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GlyphReadiness {
    Idle,
    Ready,
    Invisible,
    Failed,
}

impl From<&TextBuildStats> for GlyphReadiness {
    fn from(stats: &TextBuildStats) -> Self {
        if stats.failed_glyphs > 0 {
            Self::Failed
        } else if stats.glyphs > 0 && stats.invisible_glyphs == stats.glyphs {
            Self::Invisible
        } else if stats.glyphs > 0 && stats.ready_glyphs + stats.invisible_glyphs == stats.glyphs {
            Self::Ready
        } else {
            Self::Idle
        }
    }
}

impl TextBuildStats {
    pub(super) fn accumulate(&mut self, other: &Self) {
        self.texts += other.texts;
        self.glyphs += other.glyphs;
        self.ready_glyphs += other.ready_glyphs;
        self.invisible_glyphs += other.invisible_glyphs;
        self.failed_glyphs += other.failed_glyphs;
        self.emitted_quads += other.emitted_quads;
        self.shape_ms += other.shape_ms;
        self.atlas_ms += other.atlas_ms;
    }
}

pub(super) const fn record_prepared_glyphs(
    stats: &mut TextBuildStats,
    shaped_glyphs: usize,
    emitted_quads: usize,
) {
    stats.ready_glyphs = emitted_quads;
    stats.invisible_glyphs = shaped_glyphs.saturating_sub(emitted_quads);
    stats.emitted_quads = emitted_quads;
}

pub(super) fn positioned_glyphs<'a>(
    glyphs: &'a [ShapedGlyph],
    font_registry: &'a FontRegistry,
    stats: &mut TextBuildStats,
) -> Vec<PositionedGlyph<'a>> {
    let mut positioned_glyphs = Vec::with_capacity(glyphs.len());
    for glyph in glyphs {
        let Some((font, collection_index)) = font_registry.font_for_face(glyph.font_face) else {
            stats.failed_glyphs += 1;
            continue;
        };
        positioned_glyphs.push(PositionedGlyph {
            glyph,
            font,
            collection_index,
        });
    }
    positioned_glyphs
}

/// Runs parley text shaping, using the cache when possible.
pub(super) fn shape_text_cached(
    text: &str,
    config: &TextStyle,
    font_registry: &FontRegistry,
    shaping_cx: &TextShapingContext,
    cache: &ShapedTextCache,
) -> ShapedTextRun {
    let measure = config.as_measure();

    if let Some(cached) = cache.get_shaped(text, &measure) {
        return cached;
    }

    let font_context = font_registry.font_context();
    let mut font_context = font_context.lock().unwrap_or_else(PoisonError::into_inner);
    let mut layout_context = shaping_cx
        .layout_cx
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut layout = shaping_cx
        .layout
        .lock()
        .unwrap_or_else(PoisonError::into_inner);

    let family_name = font_registry
        .family_name(FontId(config.font_id()))
        .unwrap_or(DEFAULT_FAMILY);

    let mut builder = layout_context.ranged_builder(&mut font_context, text, 1.0, true);
    apply_text_style(&mut builder, config, family_name);

    builder.build_into(&mut layout, text);
    layout.break_all_lines(None);

    drop(font_context);
    drop(layout_context);

    let shaped_text_run = collect_shaped_run(&layout, config.font_id());
    let dimensions = text_dimensions(&layout, config);
    drop(layout);
    cache.insert_shaped(text, &measure, shaped_text_run.clone(), dimensions);
    shaped_text_run
}

fn apply_text_style(builder: &mut RangedBuilder<'_, ()>, config: &TextStyle, family: &str) {
    builder.push_default(StyleProperty::FontSize(config.size()));
    builder.push_default(StyleProperty::FontFamily(parley::style::FontFamily::named(
        family,
    )));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(
        config.weight().0,
    )));
    builder.push_default(StyleProperty::FontStyle(font_style(config.slant())));
    if config.letter_spacing() != 0.0 {
        builder.push_default(StyleProperty::LetterSpacing(config.letter_spacing()));
    }
    if config.word_spacing() != 0.0 {
        builder.push_default(StyleProperty::WordSpacing(config.word_spacing()));
    }
    if config.line_height_raw() > 0.0 {
        builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(
            config.line_height_raw(),
        )));
    }
    push_font_features(builder, config.font_features());
}

const fn font_style(slant: FontSlant) -> FontStyle {
    match slant {
        FontSlant::Normal => FontStyle::Normal,
        FontSlant::Italic => FontStyle::Italic,
        FontSlant::Oblique => FontStyle::Oblique(None),
    }
}

fn push_font_features(builder: &mut RangedBuilder<'_, ()>, font_features: crate::FontFeatures) {
    if font_features.is_default() {
        return;
    }
    let parley_features: Vec<parley::style::FontFeature> = font_features
        .to_parley_settings()
        .into_iter()
        .map(|(tag, value)| parley::FontFeature {
            tag: parley::setting::Tag::from_bytes(tag),
            value,
        })
        .collect();
    builder.push_default(StyleProperty::FontFeatures(FontFeatures::List(Cow::Owned(
        parley_features,
    ))));
}

fn collect_shaped_run(layout: &Layout<()>, requested_font_id: u16) -> ShapedTextRun {
    ShapedTextRun {
        glyphs:       collect_glyphs(layout, requested_font_id),
        line_metrics: collect_line_metrics(layout),
    }
}

fn collect_line_metrics(layout: &Layout<()>) -> Vec<LineMetricsSnapshot> {
    layout
        .lines()
        .map(|line| {
            let metrics = line.metrics();
            LineMetricsSnapshot {
                ascent:   metrics.ascent,
                descent:  metrics.descent,
                baseline: metrics.baseline,
                top:      metrics.block_min_coord,
                bottom:   metrics.block_max_coord,
            }
        })
        .collect()
}

fn collect_glyphs(layout: &Layout<()>, requested_font_id: u16) -> Vec<ShapedGlyph> {
    let mut glyphs = Vec::new();
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            append_run_glyphs(&mut glyphs, &run, requested_font_id);
        }
    }
    glyphs
}

fn append_run_glyphs(
    glyphs: &mut Vec<ShapedGlyph>,
    run: &GlyphRun<'_, ()>,
    requested_font_id: u16,
) {
    let glyph_run = run.run();
    let font = glyph_run.font();
    let font_face = ResolvedFontFace {
        requested_font_id,
        blob_id: font.data.id(),
        collection_index: font.index,
    };
    let mut advance_x = 0.0_f32;
    for cluster in glyph_run.clusters() {
        for glyph in cluster.glyphs() {
            glyphs.push(ShapedGlyph {
                font_face,
                id: glyph.id.to_u16(),
                x: run.offset() + advance_x + glyph.x,
                y: glyph.y,
                baseline: run.baseline(),
                advance: glyph.advance,
            });
            advance_x += glyph.advance;
        }
    }
}

fn text_dimensions(layout: &Layout<()>, config: &TextStyle) -> TextDimensions {
    TextDimensions {
        width:       layout.full_width(),
        height:      layout.height(),
        line_height: layout
            .lines()
            .next()
            .map_or_else(|| config.size(), |line| line.metrics().line_height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_reports_failed_takes_priority() {
        let stats = TextBuildStats {
            glyphs: 2,
            ready_glyphs: 1,
            failed_glyphs: 1,
            ..Default::default()
        };

        assert_eq!(GlyphReadiness::from(&stats), GlyphReadiness::Failed);
    }

    #[test]
    fn readiness_reports_invisible_when_all_glyphs_have_no_quad() {
        let stats = TextBuildStats {
            glyphs: 2,
            invisible_glyphs: 2,
            ..Default::default()
        };

        assert_eq!(GlyphReadiness::from(&stats), GlyphReadiness::Invisible);
    }

    #[test]
    fn prepared_glyph_stats_classify_space_only_runs_as_invisible() {
        let mut stats = TextBuildStats {
            glyphs: 1,
            ..Default::default()
        };

        record_prepared_glyphs(&mut stats, 1, 0);

        assert_eq!(GlyphReadiness::from(&stats), GlyphReadiness::Invisible);
    }

    #[test]
    fn readiness_reports_ready_for_mixed_visible_and_invisible_glyphs() {
        let stats = TextBuildStats {
            glyphs: 2,
            ready_glyphs: 1,
            invisible_glyphs: 1,
            ..Default::default()
        };

        assert_eq!(GlyphReadiness::from(&stats), GlyphReadiness::Ready);
    }
}
