//! Parley-backed text measurement and the [`DiegeticTextMeasurer`] resource.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use bevy::prelude::Resource;
use bevy_kana::ToF32;
use parley::FontContext;
use parley::Layout;
use parley::LayoutContext;
use parley::style::FontFamily;
use parley::style::FontFeatures;
use parley::style::FontStyle;
use parley::style::FontWeight;
use parley::style::LineHeight;
use parley::style::StyleProperty;

use super::constants::DEFAULT_FAMILY;
use crate::constants::MONOSPACE_WIDTH_RATIO;
use crate::layout::FontSlant;
use crate::layout::MeasureTextFn;
use crate::layout::TextDimensions;
use crate::layout::TextMeasure;

/// Resource providing text measurement for layout computation.
///
/// Insert before adding [`DiegeticUiPlugin`](crate::DiegeticUiPlugin) to
/// override the default monospace approximation. The plugin replaces the
/// default with a parley-backed measurer when it initializes the font
/// registry.
///
/// The default measurer estimates text dimensions using a fixed character
/// width (60% of font size). Custom measurers are useful when bridging
/// to external layout engines. See the `side_by_side` example for a
/// real-world case where clay-layout delegates measurement through this
/// interface.
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
                let width = char_width * text.len().to_f32();
                TextDimensions {
                    width,
                    height: measure.size,
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
#[must_use]
pub fn create_parley_measurer(
    font_cx: Arc<Mutex<FontContext>>,
    families: Vec<String>,
) -> MeasureTextFn {
    let layout_cx: Mutex<LayoutContext<()>> = Mutex::new(LayoutContext::default());
    let layout_buf: Mutex<Layout<()>> = Mutex::new(Layout::new());

    Arc::new(move |text: &str, measure: &TextMeasure| {
        let family_name = families
            .get(usize::from(measure.font_id))
            .map_or(DEFAULT_FAMILY, String::as_str);

        let mut font_cx = font_cx.lock().unwrap_or_else(PoisonError::into_inner);
        let mut layout_cx = layout_cx.lock().unwrap_or_else(PoisonError::into_inner);
        let mut layout = layout_buf.lock().unwrap_or_else(PoisonError::into_inner);

        let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
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

        builder.build_into(&mut layout, text);
        layout.break_all_lines(None);

        // Drop mutable guards before reading layout results.
        drop(font_cx);
        drop(layout_cx);

        let line_height = layout
            .lines()
            .next()
            .map_or(measure.size, |l| l.metrics().line_height);

        TextDimensions {
            width: layout.full_width(),
            height: layout.height(),
            line_height,
        }
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests use unwrap for clearer failure messages"
)]
mod tests {
    use super::create_parley_measurer;
    use crate::MeasureTextFn;
    use crate::TextMeasure;
    use crate::TextStyle;
    use crate::Unit;
    use crate::text::FontRegistry;

    fn measurer() -> MeasureTextFn {
        let font_registry = FontRegistry::new().unwrap();
        create_parley_measurer(font_registry.font_context(), font_registry.family_names())
    }

    fn default_measure(size: f32) -> TextMeasure { TextStyle::new(size).as_measure() }

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
}
