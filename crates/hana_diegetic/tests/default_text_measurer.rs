//! The fallback measurer preserves explicit line breaks before fonts are ready.

use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::TextStyle;

#[test]
fn default_measurer_uses_the_widest_line_and_counts_both_lines() {
    let measurer = DiegeticTextMeasurer::default();
    let style = TextStyle::new(10.0).as_measure();

    let bounds = (measurer.measure_fn)("Hi\nWide", &style);

    assert!((bounds.width - 24.0).abs() < f32::EPSILON);
    assert!((bounds.height - 20.0).abs() < f32::EPSILON);
    assert!((bounds.line_height - 10.0).abs() < f32::EPSILON);
}
