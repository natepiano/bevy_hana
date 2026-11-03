//! Parley-backed text measurement and the [`DiegeticTextMeasurer`] resource.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use bevy::prelude::Resource;
use hana_kana::ToF32;
use hana_kana::ToU16;
use parley::FontContext;
use parley::Layout;
use parley::LayoutContext;
use parley::fontique::FamilyInfo;
use parley::fontique::SourceKind;
use parley::layout::PositionedLayoutItem;
use parley::style::FontFamily;
use parley::style::FontFeatures;
use parley::style::FontStyle;
use parley::style::FontWeight;
use parley::style::LineHeight;
use parley::style::StyleProperty;
use ttf_parser::Face;

use super::constants::DEFAULT_FAMILY;
use super::glyph_ink_extents;
use crate::constants::MONOSPACE_WIDTH_RATIO;
use crate::layout::CapHeightMeasurement;
use crate::layout::CapHeightMetrics;
use crate::layout::FontFeatures as MeasurementFontFeatures;
use crate::layout::FontSlant;
use crate::layout::MeasureCapHeightFn;
use crate::layout::MeasureRichTextFn;
use crate::layout::MeasureTextFn;
use crate::layout::ShapedTextCache;
use crate::layout::TextDimensions;
use crate::layout::TextInk;
use crate::layout::TextInkMeasurement;
use crate::layout::TextMeasure;
use crate::render::text_shaping;

/// Resource providing text measurement for layout computation.
///
/// Insert before adding [`DiegeticUiPlugin`](crate::DiegeticUiPlugin) to
/// override the default monospace approximation. The plugin replaces the
/// default with a parley-backed measurer when it initializes the font
/// registry.
///
/// The default measurer estimates text dimensions using a fixed character
/// width (60% of font size). A custom measurer bridges to an external
/// layout engine; the `side_by_side` example has clay-layout delegate
/// measurement through this interface.
///
/// # Example
///
/// ```ignore
/// app.insert_resource(DiegeticTextMeasurer {
///     measure_fn: Arc::new(|text, measure| {
///         TextDimensions { width: 100.0, height: 12.0, line_height: 12.0 }
///     }),
/// });
/// ```
#[derive(Resource)]
pub struct DiegeticTextMeasurer {
    /// The measurement function. Takes a text string and a [`TextMeasure`]
    /// describing the font configuration, returns [`TextDimensions`].
    pub measure_fn: MeasureTextFn,
}

impl Default for DiegeticTextMeasurer {
    fn default() -> Self {
        Self {
            measure_fn: Arc::new(|text: &str, measure: &TextMeasure| {
                let char_width = measure.size * MONOSPACE_WIDTH_RATIO;
                let (widest_line, line_count) = text
                    .split('\n')
                    .fold((0_usize, 0_usize), |(widest_line, line_count), line| {
                        (widest_line.max(line.len()), line_count + 1)
                    });
                TextDimensions {
                    width:       char_width * widest_line.to_f32(),
                    height:      measure.size * line_count.to_f32(),
                    line_height: measure.size,
                }
            }),
        }
    }
}

/// Creates a [`MeasureTextFn`] backed by parley's layout engine.
///
/// Uses the shared [`FontContext`] from the registry and creates its own
/// [`LayoutContext`] and [`Layout`] buffer for text measurement.
///
/// The returned closure is `Send + Sync` via `Arc<Mutex<>>` wrappers
/// on the mutable parley contexts.
/// Repeated measurements reuse exact dimensions while the requested family's
/// faces remain unchanged. Layouts using fallback or missing glyphs are rebuilt.
#[must_use]
pub fn create_parley_measurer(
    font_cx: Arc<Mutex<FontContext>>,
    families: Vec<String>,
) -> MeasureTextFn {
    let measurements = Mutex::new(FontLayoutMeasurements::default());

    Arc::new(move |text: &str, measure: &TextMeasure| {
        let family_name = families
            .get(usize::from(measure.font_id))
            .map_or(DEFAULT_FAMILY, String::as_str);

        let mut font_cx = font_cx.lock().unwrap_or_else(PoisonError::into_inner);
        measurements
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .measure(&mut font_cx, family_name, text, measure)
    })
}

/// Measures the selected face's cap height against Parley's actual baseline.
/// Shapes the actual label so hard breaks retain the block's line-box placement.
pub(crate) fn create_cap_height_measurer(
    font_cx: Arc<Mutex<FontContext>>,
    families: Vec<String>,
) -> MeasureCapHeightFn {
    let measurements = Mutex::new(CapHeightMeasurements::default());
    Arc::new(move |text, measure| {
        let family = families
            .get(usize::from(measure.font_id))
            .map_or(DEFAULT_FAMILY, String::as_str);
        let mut font_cx = font_cx.lock().unwrap_or_else(PoisonError::into_inner);
        measurements
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .measure(&mut font_cx, family, text, measure)
    })
}

#[derive(Default)]
struct CapHeightMeasurements {
    layout_cx:  LayoutContext<()>,
    layout:     Layout<()>,
    styles:     HashMap<FontMeasurementStyle, FamilyCapMeasurements>,
    entries:    usize,
    text_bytes: usize,
}

struct FamilyCapMeasurements {
    family: FamilyInfo,
    text:   HashMap<String, CapHeightMeasurement>,
}

impl CapHeightMeasurements {
    fn measure(
        &mut self,
        context: &mut FontContext,
        family_name: &str,
        text: &str,
        measure: &TextMeasure,
    ) -> CapHeightMeasurement {
        let Some(family) = context.collection.family_by_name(family_name) else {
            return CapHeightMeasurement::LineBoxFallback;
        };
        let style = FontMeasurementStyle::from(measure);
        if let Some(previous) = self.styles.get(&style)
            && std::ptr::eq(previous.family.fonts(), family.fonts())
            && let Some(metrics) = previous.text.get(text)
        {
            return *metrics;
        }
        build_layout(
            context,
            &mut self.layout_cx,
            &mut self.layout,
            family_name,
            text,
            measure,
        );
        let shaped_line_count = self.layout.lines().count();
        let metrics = self
            .layout
            .lines()
            .next()
            .and_then(|line| {
                line.items().find_map(|item| {
                    let PositionedLayoutItem::GlyphRun(run) = item else {
                        return None;
                    };
                    let font = run.run().font();
                    let face = Face::parse(font.data.data(), font.index).ok()?;
                    let cap = face
                        .capital_height()
                        .filter(|height| *height > 0)
                        .or_else(|| {
                            face.glyph_index('H')
                                .and_then(|id| face.glyph_bounding_box(id))
                                .map(|bounds| bounds.y_max)
                        })?;
                    Some(CapHeightMetrics {
                        baseline: run.baseline(),
                        cap_height: f32::from(cap) * run.run().font_size()
                            / f32::from(face.units_per_em()),
                        shaped_line_count,
                    })
                })
            })
            .map_or(
                CapHeightMeasurement::LineBoxFallback,
                CapHeightMeasurement::FontMetrics,
            );
        if self.entries >= MAX_FONT_MEASUREMENTS
            || self.text_bytes + text.len() > MAX_MEASUREMENT_TEXT_BYTES
        {
            self.styles.clear();
            self.entries = 0;
            self.text_bytes = 0;
        }
        if text.len() <= MAX_MEASUREMENT_TEXT_BYTES {
            let previous = self
                .styles
                .entry(style)
                .or_insert_with(|| FamilyCapMeasurements {
                    family: family.clone(),
                    text:   HashMap::new(),
                });
            if !std::ptr::eq(previous.family.fonts(), family.fonts()) {
                self.entries -= previous.text.len();
                self.text_bytes -= previous.text.keys().map(String::len).sum::<usize>();
                previous.family = family;
                previous.text.clear();
            }
            previous.text.insert(text.to_owned(), metrics);
            self.entries += 1;
            self.text_bytes += text.len();
        }
        metrics
    }
}

/// Exact shaping properties: nearby sizes must not share rounded measurements.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct FontMeasurementStyle {
    font_id:             u16,
    size_bits:           u32,
    weight_bits:         u32,
    slant:               u8,
    line_height_bits:    u32,
    letter_spacing_bits: u32,
    word_spacing_bits:   u32,
    features:            MeasurementFontFeatures,
}

impl From<&TextMeasure> for FontMeasurementStyle {
    fn from(measure: &TextMeasure) -> Self {
        Self {
            font_id:             measure.font_id,
            size_bits:           measure.size.to_bits(),
            weight_bits:         measure.weight.0.to_bits(),
            slant:               match measure.slant {
                FontSlant::Normal => 0,
                FontSlant::Italic => 1,
                FontSlant::Oblique => 2,
            },
            line_height_bits:    measure.line_height.to_bits(),
            letter_spacing_bits: measure.letter_spacing.to_bits(),
            word_spacing_bits:   measure.word_spacing.to_bits(),
            features:            measure.font_features,
        }
    }
}

/// Measurements whose glyphs all came from this immutable family version.
struct FamilyTextMeasurements {
    family: FamilyInfo,
    text:   HashMap<String, TextDimensions>,
}

/// Reuses layout buffers and measurements across native construction and layout.
#[derive(Default)]
struct FontLayoutMeasurements {
    layout_cx:     LayoutContext<()>,
    layout:        Layout<()>,
    styles:        HashMap<FontMeasurementStyle, FamilyTextMeasurements>,
    entries:       usize,
    text_bytes:    usize,
    #[cfg(test)]
    layout_builds: usize,
}

const MAX_FONT_MEASUREMENTS: usize = 512;
const MAX_MEASUREMENT_TEXT_BYTES: usize = 1024 * 1024;

impl FontLayoutMeasurements {
    fn measure(
        &mut self,
        font_cx: &mut FontContext,
        family_name: &str,
        text: &str,
        measure: &TextMeasure,
    ) -> TextDimensions {
        let style = FontMeasurementStyle::from(measure);
        let family = font_cx.collection.family_by_name(family_name);
        if let Some(family) = &family
            && let Some(previous) = self.styles.get(&style)
            // FamilyInfo keeps the old slice alive; replacing any face creates
            // a new slice, so even same-name font registration invalidates it.
            && std::ptr::eq(previous.family.fonts(), family.fonts())
            && let Some(dimensions) = previous.text.get(text)
        {
            return *dimensions;
        }

        build_layout(
            font_cx,
            &mut self.layout_cx,
            &mut self.layout,
            family_name,
            text,
            measure,
        );
        #[cfg(test)]
        {
            self.layout_builds += 1;
        }
        let dimensions = TextDimensions {
            width:       self.layout.full_width(),
            height:      self.layout.height(),
            line_height: self
                .layout
                .lines()
                .next()
                .map_or(measure.size, |line| line.metrics().line_height),
        };
        if let Some(family) = family
            && text.len() <= MAX_MEASUREMENT_TEXT_BYTES
            && self.uses_only_family(&family)
        {
            self.remember(style, family, text, dimensions);
        }
        dimensions
    }

    fn uses_only_family(&self, family: &FamilyInfo) -> bool {
        // Fallbacks and missing glyphs can change independently of the requested
        // family. Keep measuring them rather than returning stale dimensions.
        !family.fonts().is_empty()
            && family.fonts().iter().all(|face| matches!(face.source().kind(), SourceKind::Memory(_)))
            && self.layout.lines().flat_map(|line| line.items()).all(|item| {
            let PositionedLayoutItem::GlyphRun(run) = item else { return true; };
            let font = run.run().font();
            family.fonts().iter().any(|face| {
                face.index() == font.index
                    && matches!(face.source().kind(), SourceKind::Memory(blob) if blob.id() == font.data.id())
            }) && run.run().clusters().flat_map(|cluster| cluster.glyphs()).all(|glyph| glyph.id != 0)
        })
    }

    fn remember(
        &mut self,
        style: FontMeasurementStyle,
        family: FamilyInfo,
        text: &str,
        dimensions: TextDimensions,
    ) {
        // Tool labels repeat, but arbitrary edits must not grow this cache forever.
        if self.entries >= MAX_FONT_MEASUREMENTS
            || self.text_bytes + text.len() > MAX_MEASUREMENT_TEXT_BYTES
        {
            self.styles.clear();
            self.entries = 0;
            self.text_bytes = 0;
        }
        let previous = self
            .styles
            .entry(style)
            .or_insert_with(|| FamilyTextMeasurements {
                family: family.clone(),
                text:   HashMap::new(),
            });
        if !std::ptr::eq(previous.family.fonts(), family.fonts()) {
            self.entries -= previous.text.len();
            self.text_bytes -= previous.text.keys().map(String::len).sum::<usize>();
            previous.family = family;
            previous.text.clear();
        }
        previous.text.insert(text.to_owned(), dimensions);
        self.entries += 1;
        self.text_bytes += text.len();
    }
}

/// Lays out `text` in `measure`'s style and measures the ink its glyphs draw.
pub(super) fn measure_ink(
    font_cx: &Mutex<FontContext>,
    family_name: &str,
    text: &str,
    measure: &TextMeasure,
) -> TextInkMeasurement {
    let mut layout: Layout<()> = Layout::new();
    let mut font_cx = font_cx.lock().unwrap_or_else(PoisonError::into_inner);
    build_layout(
        &mut font_cx,
        &mut LayoutContext::default(),
        &mut layout,
        family_name,
        text,
        measure,
    );
    drop(font_cx);

    let mut ink = TextInkMeasurement::NoVisibleInk;
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let font = run.run().font();
            let Ok(face) = Face::parse(font.data.data(), font.index) else {
                continue;
            };
            let font_size = run.run().font_size();
            let mut advance_x = 0.0;
            for cluster in run.run().clusters() {
                for glyph in cluster.glyphs() {
                    let Some(extents) = glyph_ink_extents(&face, glyph.id.to_u16()) else {
                        advance_x += glyph.advance;
                        continue;
                    };
                    let scale = font_size / extents.units_per_em;
                    let x = run.offset() + advance_x + glyph.x;
                    let baseline = run.baseline() + glyph.y;
                    let glyph_ink = TextInk {
                        left:   extents.min_x.mul_add(scale, x),
                        top:    extents.max_y.mul_add(-scale, baseline),
                        right:  extents.max_x.mul_add(scale, x),
                        bottom: extents.min_y.mul_add(-scale, baseline),
                    };
                    ink = TextInkMeasurement::Visible(match ink {
                        TextInkMeasurement::NoVisibleInk => glyph_ink,
                        TextInkMeasurement::Visible(ink) => TextInk {
                            left:   ink.left.min(glyph_ink.left),
                            top:    ink.top.min(glyph_ink.top),
                            right:  ink.right.max(glyph_ink.right),
                            bottom: ink.bottom.max(glyph_ink.bottom),
                        },
                    });
                    advance_x += glyph.advance;
                }
            }
        }
    }
    ink
}

/// Shapes `text` into `layout` with every measurement property of `measure`.
pub(super) fn build_layout(
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    layout: &mut Layout<()>,
    family_name: &str,
    text: &str,
    measure: &TextMeasure,
) {
    let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::FontSize(measure.size));
    builder.push_default(StyleProperty::FontFamily(FontFamily::named(family_name)));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(measure.weight.0)));

    let font_style = match measure.slant {
        FontSlant::Normal => FontStyle::Normal,
        FontSlant::Italic => FontStyle::Italic,
        FontSlant::Oblique => FontStyle::Oblique(None),
    };
    builder.push_default(StyleProperty::FontStyle(font_style));

    if measure.letter_spacing != 0.0 {
        builder.push_default(StyleProperty::LetterSpacing(measure.letter_spacing));
    }
    if measure.word_spacing != 0.0 {
        builder.push_default(StyleProperty::WordSpacing(measure.word_spacing));
    }

    if measure.line_height > 0.0 {
        builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(
            measure.line_height,
        )));
    }

    // Push OpenType feature overrides (liga, calt, dlig, kern).
    let font_features = measure.font_features;
    if !font_features.is_default() {
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

    builder.build_into(layout, text);
    layout.break_all_lines(None);
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests use unwrap for clearer failure messages"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use super::CapHeightMeasurement;
    use super::FontLayoutMeasurements;
    use super::MAX_FONT_MEASUREMENTS;
    use super::create_cap_height_measurer;
    use super::create_parley_measurer;
    use crate::FontFeatureFlags;
    use crate::FontSlant;
    use crate::MeasureTextFn;
    use crate::TextInk;
    use crate::TextInkMeasurement;
    use crate::TextMeasure;
    use crate::TextStyle;
    use crate::Unit;
    use crate::text::FontId;
    use crate::text::FontRegistry;
    use crate::text::font::constants::DEFAULT_FAMILY;
    use crate::text::font::constants::EMBEDDED_BOLD_FONT;

    fn measurer() -> MeasureTextFn {
        let font_registry = FontRegistry::new().unwrap();
        create_parley_measurer(font_registry.font_context(), font_registry.family_names())
    }

    #[test]
    fn cap_height_metrics_follow_drawn_baseline_at_small_sizes_and_explicit_leading() {
        let registry = FontRegistry::new().unwrap();
        let cap_measure =
            create_cap_height_measurer(registry.font_context(), registry.family_names());
        for size in [8.0, 11.0, 14.0, 23.5] {
            for leading in [0.0, 32.0] {
                let style = TextStyle::new(size).with_line_height(leading);
                let CapHeightMeasurement::FontMetrics(metrics) =
                    cap_measure("H", &style.as_measure())
                else {
                    panic!("bundled font must provide cap metrics");
                };
                let ink = visible_ink(&registry, "H", &style.as_measure());
                assert!((metrics.baseline - ink.bottom).abs() < 0.001);
                assert!((metrics.baseline - metrics.cap_height - ink.top).abs() < 0.001);
            }
        }
    }

    fn visible_ink(registry: &FontRegistry, text: &str, measure: &TextMeasure) -> TextInk {
        match registry.measure_ink(text, measure).unwrap() {
            TextInkMeasurement::Visible(ink) => ink,
            TextInkMeasurement::NoVisibleInk => panic!("{text:?} should draw ink"),
        }
    }

    fn default_measure(size: f32) -> TextMeasure { TextStyle::new(size).as_measure() }

    #[test]
    fn repeated_layout_measurements_reuse_every_exact_shaping_input() {
        let registry = FontRegistry::new().unwrap();
        let context = registry.font_context();
        let mut context = context.lock().unwrap();
        let mut measurements = FontLayoutMeasurements::default();
        let base = default_measure(16.0);
        let mut styles = vec![base; 9];
        styles[1].font_id = 999;
        styles[2].size = f32::from_bits(base.size.to_bits() + 1);
        styles[3].weight.0 = 700.0;
        styles[4].slant = FontSlant::Italic;
        styles[5].line_height = 24.0;
        styles[6].letter_spacing = 2.0;
        styles[7].word_spacing = 3.0;
        styles[8].font_features = base.font_features.without(FontFeatureFlags::LIGA);
        for (index, style) in styles.iter().enumerate() {
            let dimensions = measurements.measure(&mut context, DEFAULT_FAMILY, "ffi tool", style);
            let mut fresh = FontLayoutMeasurements::default();
            assert_eq!(
                dimensions,
                fresh.measure(&mut context, DEFAULT_FAMILY, "ffi tool", style)
            );
            assert_eq!(measurements.layout_builds, index + 1);
            assert_eq!(
                dimensions,
                measurements.measure(&mut context, DEFAULT_FAMILY, "ffi tool", style)
            );
            assert_eq!(
                measurements.layout_builds,
                index + 1,
                "repeat should reuse {style:?}"
            );
        }
        measurements.measure(&mut context, DEFAULT_FAMILY, "another tool", &base);
        drop(context);
        assert_eq!(measurements.layout_builds, styles.len() + 1);
    }

    #[test]
    fn replacing_same_family_faces_invalidates_measurements() {
        let mut registry = FontRegistry::new().unwrap();
        let context = registry.font_context();
        let measure = default_measure(16.0);
        let mut measurements = FontLayoutMeasurements::default();
        {
            let mut context = context.lock().unwrap();
            measurements.measure(&mut context, DEFAULT_FAMILY, "Tool", &measure);
            let family = context.collection.family_id(DEFAULT_FAMILY).unwrap();
            assert!(context.collection.unregister_font(
                family,
                parley::fontique::FontWidth::default(),
                parley::fontique::FontStyle::default(),
                parley::fontique::FontWeight::default()
            ));
            drop(context);
        }
        registry
            .register_font(DEFAULT_FAMILY, EMBEDDED_BOLD_FONT)
            .unwrap();
        let mut context = context.lock().unwrap();
        let dimensions = measurements.measure(&mut context, DEFAULT_FAMILY, "Tool", &measure);
        assert_eq!(measurements.layout_builds, 2);
        assert_eq!(
            dimensions,
            FontLayoutMeasurements::default().measure(
                &mut context,
                DEFAULT_FAMILY,
                "Tool",
                &measure
            )
        );
        assert_eq!(
            dimensions,
            measurements.measure(&mut context, DEFAULT_FAMILY, "Tool", &measure)
        );
        drop(context);
        assert_eq!(measurements.layout_builds, 2);
    }

    #[test]
    fn missing_glyphs_do_not_reuse_family_measurements() {
        let registry = FontRegistry::new().unwrap();
        let context = registry.font_context();
        let mut context = context.lock().unwrap();
        let mut measurements = FontLayoutMeasurements::default();
        let measure = default_measure(16.0);
        let first = measurements.measure(&mut context, DEFAULT_FAMILY, "\u{10ffff}", &measure);
        assert_eq!(
            first,
            measurements.measure(&mut context, DEFAULT_FAMILY, "\u{10ffff}", &measure)
        );
        drop(context);
        assert_eq!(measurements.layout_builds, 2);
        assert_eq!(measurements.entries, 0);
    }

    #[test]
    fn changing_label_text_keeps_measurement_storage_bounded() {
        let registry = FontRegistry::new().unwrap();
        let context = registry.font_context();
        let mut context = context.lock().unwrap();
        let mut measurements = FontLayoutMeasurements::default();
        let measure = default_measure(16.0);
        for index in 0..=MAX_FONT_MEASUREMENTS {
            measurements.measure(
                &mut context,
                DEFAULT_FAMILY,
                &format!("Tool {index}"),
                &measure,
            );
            assert!(measurements.entries <= MAX_FONT_MEASUREMENTS);
        }
        let builds = measurements.layout_builds;
        measurements.measure(&mut context, DEFAULT_FAMILY, "Tool 0", &measure);
        assert_eq!(
            measurements.layout_builds,
            builds + 1,
            "old labels should be evicted"
        );
    }

    #[test]
    fn measures_nonzero_dimensions() {
        let measure = measurer();
        let dims = measure("Hello", &default_measure(16.0));
        assert!(
            dims.width > 0.0,
            "width should be positive, got {}",
            dims.width
        );
        assert!(
            dims.height > 0.0,
            "height should be positive, got {}",
            dims.height
        );
    }

    #[test]
    fn measures_nonzero_width_at_world_scale() {
        // WorldText `.size(0.16)` is meters (bare f32); the panel scales it to
        // points by `Unit::Meters.to_points()` (~2835x) before measuring.
        let measure = measurer();
        let size = 0.16 * Unit::Meters.to_points();
        let dims = measure("Hello", &default_measure(size));
        assert!(
            dims.width > 0.0,
            "width at {size}pt should be positive, got {}",
            dims.width
        );
    }

    #[test]
    fn empty_string_is_narrower_than_content() {
        let measure = measurer();
        let m = default_measure(16.0);
        let empty = measure("", &m);
        let content = measure("Hello", &m);
        assert!(
            empty.width < content.width,
            "empty string should be narrower than content: {:.2} vs {:.2}",
            empty.width,
            content.width
        );
    }

    #[test]
    fn longer_text_is_wider() {
        let measure = measurer();
        let m = default_measure(16.0);
        let short = measure("Hi", &m);
        let long = measure("Hello, world!", &m);
        assert!(
            long.width > short.width,
            "longer text should be wider: {:.2} vs {:.2}",
            long.width,
            short.width
        );
    }

    #[test]
    fn larger_font_produces_wider_text() {
        let measure = measurer();
        let small = measure("Hello", &default_measure(10.0));
        let large = measure("Hello", &default_measure(20.0));
        assert!(
            large.width > small.width,
            "20pt should be wider than 10pt: {:.2} vs {:.2}",
            large.width,
            small.width
        );
    }

    #[test]
    fn larger_font_produces_taller_text() {
        let measure = measurer();
        let small = measure("Hello", &default_measure(10.0));
        let large = measure("Hello", &default_measure(20.0));
        assert!(
            large.height > small.height,
            "20pt should be taller than 10pt: {:.2} vs {:.2}",
            large.height,
            small.height
        );
    }

    #[test]
    fn width_scales_roughly_with_font_size() {
        let measure = measurer();
        let small = measure("Hello", &default_measure(10.0));
        let large = measure("Hello", &default_measure(20.0));
        let ratio = large.width / small.width;
        assert!(
            (1.5..2.5).contains(&ratio),
            "2x font size should roughly double width, got ratio {ratio:.2}"
        );
    }

    #[test]
    fn monospace_equal_length_strings_have_similar_width() {
        let measure = measurer();
        let m = default_measure(16.0);
        let a = measure("iiiii", &m);
        let b = measure("MMMMM", &m);
        let diff = (a.width - b.width).abs();
        assert!(
            diff < 1.0,
            "monospace font: 'iiiii' and 'MMMMM' should have similar width, diff={diff:.2}"
        );
    }

    #[test]
    fn bold_text_is_at_least_as_wide() {
        let measure = measurer();
        let normal = measure("Hello", &default_measure(16.0));
        let bold_measure = TextStyle::new(16.0).bold().as_measure();
        let bold = measure("Hello", &bold_measure);
        assert!(
            bold.width >= normal.width - 0.5,
            "bold should not be narrower: bold={:.2} normal={:.2}",
            bold.width,
            normal.width
        );
    }

    #[test]
    fn newline_increases_height() {
        let measure = measurer();
        let m = default_measure(16.0);
        let one_line = measure("Hello", &m);
        let two_lines = measure("Hello\nWorld", &m);
        assert!(
            two_lines.height > one_line.height,
            "two lines should be taller: {:.2} vs {:.2}",
            two_lines.height,
            one_line.height
        );
    }

    #[test]
    fn ink_without_descenders_stops_at_the_baseline() {
        // Flat-bottomed capitals: round letters overshoot the baseline slightly.
        let registry = FontRegistry::new().unwrap();
        let measure = default_measure(100.0);
        let metrics = registry.font(FontId::MONOSPACE).unwrap().metrics(100.0);
        let line_box = measurer()("HIT", &measure);
        let baseline =
            (line_box.height - metrics.ascent - metrics.descent).mul_add(0.5, metrics.ascent);
        let ink = visible_ink(&registry, "HIT", &measure);
        assert!(
            (ink.bottom - baseline).abs() < 0.5,
            "ink bottom {} should sit on the baseline {baseline}",
            ink.bottom
        );
        assert!(
            ink.top > 0.0,
            "ink top {} should sit below the line top",
            ink.top
        );
        assert!(ink.left >= 0.0 && ink.right <= line_box.width);
    }

    #[test]
    fn world_size_ink_measured_in_renderer_points_sits_inside_its_line_box() {
        let registry = FontRegistry::new().unwrap();
        let measure = default_measure(0.0153 * Unit::Meters.to_points());
        let line_box = measurer()("HELP", &measure);
        let ink = visible_ink(&registry, "HELP", &measure);
        assert!(ink.top > 0.0 && ink.top < ink.bottom);
        assert!(ink.bottom < line_box.height);
    }

    #[test]
    fn descenders_reach_below_the_baseline() {
        let registry = FontRegistry::new().unwrap();
        let measure = default_measure(100.0);
        let line_box = measurer()("gy", &measure);
        let plain = visible_ink(&registry, "Text", &measure);
        let descending = visible_ink(&registry, "gy", &measure);
        assert!(
            descending.bottom > plain.bottom + 10.0,
            "descender bottom {} should reach well below the baseline {}",
            descending.bottom,
            plain.bottom
        );
        assert!(descending.bottom <= line_box.height);
    }

    #[test]
    fn the_last_line_holds_the_lowest_ink() {
        let registry = FontRegistry::new().unwrap();
        let measure = default_measure(100.0);
        let one_line = visible_ink(&registry, "Text", &measure);
        let two_lines = visible_ink(&registry, "Text\nText", &measure);
        let line_height = measurer()("Text", &measure).line_height;
        assert!(
            (two_lines.bottom - one_line.bottom - line_height).abs() < 0.5,
            "second line ink bottom {} should sit one line below {}",
            two_lines.bottom,
            one_line.bottom
        );
        assert!((two_lines.top - one_line.top).abs() < 0.5);
    }

    #[test]
    fn blank_text_draws_no_ink() {
        let registry = FontRegistry::new().unwrap();
        let measure = default_measure(100.0);
        assert_eq!(
            registry.measure_ink("", &measure),
            Ok(TextInkMeasurement::NoVisibleInk)
        );
        assert_eq!(
            registry.measure_ink("   ", &measure),
            Ok(TextInkMeasurement::NoVisibleInk)
        );
    }

    #[test]
    fn unknown_font_id_still_measures() {
        let measure = measurer();
        let mut m = default_measure(16.0);
        m.font_id = 999;
        let dims = measure("Hello", &m);
        assert!(
            dims.width > 0.0,
            "unknown font_id should still measure, got width {}",
            dims.width
        );
    }

    #[test]
    fn explicit_line_pitch_spaces_each_ink_baseline() {
        let registry = FontRegistry::new().unwrap();
        let font = registry.font(FontId::MONOSPACE).unwrap();
        let natural = font.metrics(11.0).line_height;
        let pitch = (natural + 1.0).ceil();
        let style = TextStyle::new(11.0)
            .with_font(FontId::MONOSPACE.0)
            .with_line_height(pitch);
        let measure = style.as_measure();
        assert!((style.line_height_raw() - pitch).abs() < 0.01);
        assert!((measure.line_height - pitch).abs() < 0.01);

        let line = "Text";
        let single_ink = visible_ink(&registry, line, &measure);
        let origins = [line, "Text\nText", "Text\nText\nText"]
            .map(|prefix| visible_ink(&registry, prefix, &measure).bottom - single_ink.bottom);
        for pair in origins.windows(2) {
            assert!(
                (pair[1] - pair[0] - pitch).abs() < 0.1,
                "{origins:?}, pitch {pitch}"
            );
        }
    }
}

/// Paragraph measurement shares its shaped geometry with rich reification.
#[derive(Resource)]
pub(crate) struct RichTextMeasurer {
    pub(crate) measure_fn: MeasureRichTextFn,
}

pub(crate) fn create_parley_rich_measurer(
    font_cx: Arc<Mutex<FontContext>>,
    families: Vec<String>,
    cache: ShapedTextCache,
) -> MeasureRichTextFn {
    let context = crate::render::text_shaping::TextShapingContext::default();
    Arc::new(move |paragraph, scale| {
        let family = families
            .get(usize::from(paragraph.base.font_id()))
            .map_or(DEFAULT_FAMILY, String::as_str);
        text_shaping::shape_rich_with_fonts(paragraph, scale, &font_cx, family, &context, &cache)
            .dimensions
    })
}
