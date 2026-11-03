//! The public text outline contract: `FontRegistry::text_outline` from outside
//! the crate, headless.
#![allow(
    clippy::expect_used,
    reason = "a font fixture that fails to build should fail the test immediately"
)]
#![allow(clippy::panic, reason = "tests should panic on unexpected values")]

use bevy::math::Vec2;
use hana_diegetic::FontId;
use hana_diegetic::FontLookupError;
use hana_diegetic::FontRegistry;
use hana_diegetic::TextMeasure;
use hana_diegetic::TextOutline;
use hana_diegetic::TextOutlineError;
use hana_diegetic::TextOutlineMeasurement;
use hana_diegetic::TextStyle;

/// Font size the outlines here are measured at.
const SIZE: f32 = 40.0;

/// Float noise allowed when one outline is compared with a scaled copy.
const SCALE_TOLERANCE: f32 = 1e-3;

fn embedded_registry() -> FontRegistry {
    FontRegistry::new().expect("the embedded font family builds")
}

fn measure(size: f32) -> TextMeasure { TextStyle::new(size).as_measure() }

fn visible(registry: &FontRegistry, text: &str, measure: &TextMeasure) -> TextOutline {
    match registry
        .text_outline(text, measure)
        .expect("the embedded family outlines")
    {
        TextOutlineMeasurement::Visible(outline) => outline,
        TextOutlineMeasurement::NoVisibleInk => panic!("{text:?} should draw ink"),
    }
}

#[test]
fn each_letter_is_one_glyph_whose_ink_lies_in_its_bounds() {
    let registry = embedded_registry();
    let outline = visible(&registry, "hana", &measure(SIZE));
    assert_eq!(outline.glyphs().len(), 4);

    for glyph in outline.glyphs() {
        let bounds = glyph.bounds();
        assert!(
            outline.bounds().contains(bounds.min) && outline.bounds().contains(bounds.max),
            "the text bounds should hold the glyph"
        );
        let beside = Vec2::new(bounds.max.x + 1.0, bounds.center().y);
        assert!(!glyph.contains(beside), "nothing past the bounds is ink");
        let has_ink = (0..=20u8).any(|step| {
            let x = bounds.min.x + bounds.width() * f32::from(step) / 20.0;
            glyph.contains(Vec2::new(x, bounds.center().y))
        });
        assert!(has_ink, "a row through the glyph should cross its ink");
    }
}

#[test]
fn units_are_layout_units_with_y_up_from_the_first_baseline() {
    let registry = embedded_registry();
    let small = visible(&registry, "hg", &measure(SIZE));
    let large = visible(&registry, "hg", &measure(SIZE * 2.0));
    let [h, g] = small.glyphs() else {
        panic!("expected two glyphs");
    };

    assert!(
        h.bounds().min.y.abs() < SIZE * 0.02,
        "h should stand on the baseline at y = 0, got {}",
        h.bounds().min.y
    );
    assert!(
        h.bounds().max.y > SIZE * 0.5,
        "h's ascender should rise above it"
    );
    assert!(
        g.bounds().min.y < -SIZE * 0.1,
        "g's descender should fall below it"
    );
    assert!(
        h.bounds().min.x >= 0.0 && h.bounds().min.x < SIZE * 0.2,
        "the first glyph should start near x = 0, got {}",
        h.bounds().min.x
    );

    let doubled = (large.bounds().min - small.bounds().min * 2.0)
        .abs()
        .max((large.bounds().max - small.bounds().max * 2.0).abs())
        .max_element();
    assert!(
        doubled < SCALE_TOLERANCE * SIZE,
        "twice the size should draw the same outline twice as large"
    );
}

#[test]
fn blank_text_has_no_visible_ink() {
    let registry = embedded_registry();
    assert_eq!(
        registry.text_outline(" ", &measure(SIZE)),
        Ok(TextOutlineMeasurement::NoVisibleInk)
    );
}

#[test]
fn an_unregistered_font_id_is_a_lookup_error() {
    let registry = embedded_registry();
    let mut unregistered = measure(SIZE);
    unregistered.font_id = u16::MAX;
    assert_eq!(
        registry.text_outline("hana", &unregistered),
        Err(TextOutlineError::Lookup(FontLookupError::UnknownFamilyId(
            FontId(u16::MAX)
        )))
    );
}
