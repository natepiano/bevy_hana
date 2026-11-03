//! One-line control text through the public panel builder and headless solver.
#![allow(
    clippy::expect_used,
    reason = "invalid headless test fixtures should fail immediately"
)]

use std::sync::Arc;

use bevy::prelude::App;
use bevy::prelude::MinimalPlugins;
use hana_diegetic::Button;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::ControlMetrics;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextDimensions;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;
use hana_diegetic::Unit;
use hana_kana::ToF32;

const LINE_HEIGHT: f32 = 13.0;
const TOLERANCE: f32 = 0.001;

fn solved_content_height(tree: LayoutTree) -> f32 {
    let panel = DiegeticPanel::world()
        .size(Px(120.0), Fit)
        .font_unit(Unit::Pixels)
        .with_tree(tree)
        .build()
        .expect("test panel should build");
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(DiegeticTextMeasurer {
            measure_fn: Arc::new(|text, _| TextDimensions {
                // This exact width exercises Fit's f32 subtraction of side padding.
                width:       if text == "Pin" {
                    20.03
                } else {
                    text.chars().count().to_f32() * 5.0
                },
                height:      LINE_HEIGHT,
                line_height: LINE_HEIGHT,
            }),
        })
        .add_plugins(HeadlessDiegeticUiPlugin);
    let panel = app.world_mut().spawn(panel).id();
    app.update();
    app.world()
        .get::<ComputedDiegeticPanel>(panel)
        .expect("panel should solve")
        .content_bounds()
        .expect("first child should have solved bounds")
        .height
}

#[test]
fn fit_button_keeps_pin_on_one_line_after_padding_rounding() {
    let metrics = ControlMetrics::from_label_size(Px(13.0));
    let mut builder = LayoutBuilder::new(120.0, 100.0);
    builder.with(
        El::row()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::xy(metrics.button.side_padding, Px(0.0)))
            .button("pin"),
        |builder| {
            builder.text(Text::new("Pin", TextStyle::new(metrics.label_size)));
        },
    );

    let height = solved_content_height(builder.build());
    assert!(
        (height - LINE_HEIGHT).abs() < TOLERANCE,
        "Fit button should hold Pin on one line; height was {height}"
    );
}

#[test]
fn fixed_width_button_truncates_a_long_label_to_one_line() {
    let mut builder = LayoutBuilder::new(120.0, 100.0);
    builder.with(
        El::row()
            .width(Sizing::fixed(Px(25.0)))
            .height(Sizing::FIT)
            .button("zoom"),
        |builder| {
            builder.text(Text::new("Zoom Out", TextStyle::new(Px(13.0))));
        },
    );

    let height = solved_content_height(builder.build());
    assert!(
        (height - LINE_HEIGHT).abs() < TOLERANCE,
        "fixed-width button should stay one line; height was {height}"
    );
}

#[test]
fn capped_fit_width_button_keeps_two_word_label_on_one_line() {
    let mut builder = LayoutBuilder::new(120.0, 100.0);
    builder.with(
        El::row()
            .width(Sizing::fit_range(Px(0.0), Px(25.0)))
            .height(Sizing::FIT)
            .button("zoom"),
        |builder| {
            builder.text(Text::new("Zoom Out", TextStyle::new(Px(13.0))));
        },
    );

    let height = solved_content_height(builder.build());
    assert!(
        (height - LINE_HEIGHT).abs() < TOLERANCE,
        "capped Fit button should keep Zoom Out on one line; height was {height}"
    );
}

#[test]
fn narrow_pop_up_value_stays_inside_one_button_line() {
    let metrics = ControlMetrics::from_label_size(Px(13.0));
    let mut builder = LayoutBuilder::new(120.0, 100.0);
    builder.text(Button::new().pop_up("choice", "Opaque violet", "Opaque violet", &metrics));

    let height = solved_content_height(builder.build());
    let button_height = metrics.button.height.0 * Unit::Pixels.to_points();
    assert!(
        (height - button_height).abs() < TOLERANCE,
        "pop-up value should stay within one button line; height was {height}"
    );
}

#[test]
fn prose_in_a_fixed_width_column_still_wraps() {
    let mut builder = LayoutBuilder::new(120.0, 100.0);
    builder.with(
        El::column()
            .width(Sizing::fixed(Px(25.0)))
            .height(Sizing::FIT),
        |builder| {
            builder.text(Text::new("Zoom Out", TextStyle::new(Px(13.0))).wrap(TextWrap::Words));
        },
    );

    let height = solved_content_height(builder.build());
    assert!(
        height > LINE_HEIGHT + TOLERANCE,
        "ordinary Words text should wrap to multiple lines; height was {height}"
    );
}
