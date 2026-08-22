//! Clay parity tests.
//!
//! Each test builds the same layout in both Clay (via `clay-layout` FFI) and
//! `hana_diegetic`, then asserts that every rectangle/text bounding box matches.
//! This is the source of truth — if Clay produces it, we should too.

#![allow(
    clippy::float_cmp,
    reason = "tests compare exact expected layout values"
)]
#![allow(
    clippy::needless_collect,
    reason = "tests collect into named variables for readable assertions and index access"
)]
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "tests use panic/unwrap for clearer failure messages"
)]

use std::sync::Arc;

use bevy_kana::ToF32;
use clay_layout::Clay;
use clay_layout::ClayLayoutScope;
use clay_layout::Declaration;
use clay_layout::fit;
use clay_layout::fixed;
use clay_layout::grow;
use clay_layout::layout::Alignment;
use clay_layout::layout::LayoutAlignmentX;
use clay_layout::layout::LayoutAlignmentY;
use clay_layout::layout::LayoutDirection;
use clay_layout::math::Dimensions;
use clay_layout::render_commands::RenderCommand;
use clay_layout::render_commands::RenderCommandConfig;
use clay_layout::text::TextConfig;

use crate::layout::AlignX;
use crate::layout::AlignY;
use crate::layout::El;
use crate::layout::LayoutBuilder;
use crate::layout::LayoutEngine;
use crate::layout::LayoutResult;
use crate::layout::MeasureTextFn;
use crate::layout::Padding;
use crate::layout::RenderCommandKind;
use crate::layout::Sizing;
use crate::layout::TextDimensions;
use crate::layout::TextMeasure;
use crate::layout::TextStyle;

// ── Shared measurement ────────────────────────────────────────────────────

const FONT_SIZE: f32 = 10.0;
const CLAY_FONT_SIZE: u16 = 10;
const CHAR_WIDTH_FACTOR: f32 = 0.6;

/// Monospace measurement: each char = `font_size * 0.6` wide, one line = `font_size` tall.
fn monospace_measure() -> MeasureTextFn {
    Arc::new(|text: &str, measure: &TextMeasure| {
        let char_width = measure.size * CHAR_WIDTH_FACTOR;
        let mut max_line_width: f32 = 0.0;
        let mut line_count = 0_u32;
        for line in text.lines() {
            line_count += 1;
            let width = line.chars().count().to_f32() * char_width;
            max_line_width = max_line_width.max(width);
        }
        if line_count == 0 {
            line_count = 1;
        }
        TextDimensions {
            width:       max_line_width,
            height:      measure.size * line_count.to_f32(),
            line_height: measure.size,
        }
    })
}

/// Same measurement logic for Clay's callback.
fn clay_monospace_measure(text: &str, config: &TextConfig, _: &mut ()) -> Dimensions {
    let font_size = f32::from(config.font_size);
    let char_width = font_size * CHAR_WIDTH_FACTOR;
    let line_height = if config.line_height == 0 {
        font_size
    } else {
        f32::from(config.line_height)
    };
    let mut max_line_width: f32 = 0.0;
    let mut line_count = 0_u32;
    for line in text.lines() {
        line_count += 1;
        let width = line.chars().count().to_f32() * char_width;
        max_line_width = max_line_width.max(width);
    }
    if line_count == 0 {
        line_count = 1;
    }
    Dimensions {
        width:  max_line_width,
        height: line_height * line_count.to_f32(),
    }
}

// ── Bounding box comparison ───────────────────────────────────────────────

/// A simplified bounding box for comparison between the two engines.
#[derive(Debug, Clone, Copy)]
struct Bbox {
    x:    f32,
    y:    f32,
    w:    f32,
    h:    f32,
    kind: BboxKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BboxKind {
    Rectangle,
    Text,
}

fn approx_eq(a: f32, b: f32) -> bool { (a - b).abs() < 0.5 }

fn assert_bboxes_match(clay_boxes: &[Bbox], diegetic_boxes: &[Bbox], kind: BboxKind) {
    let clay_filtered: Vec<_> = clay_boxes.iter().filter(|b| b.kind == kind).collect();
    let diegetic_filtered: Vec<_> = diegetic_boxes.iter().filter(|b| b.kind == kind).collect();

    assert_eq!(
        clay_filtered.len(),
        diegetic_filtered.len(),
        "{kind:?} count mismatch: Clay={}, Diegetic={}",
        clay_filtered.len(),
        diegetic_filtered.len(),
    );

    for (i, (c, d)) in clay_filtered
        .iter()
        .zip(diegetic_filtered.iter())
        .enumerate()
    {
        assert!(
            approx_eq(c.x, d.x)
                && approx_eq(c.y, d.y)
                && approx_eq(c.w, d.w)
                && approx_eq(c.h, d.h),
            "{kind:?}[{i}] mismatch:\n  Clay:     x={:.1} y={:.1} w={:.1} h={:.1}\n  Diegetic: x={:.1} y={:.1} w={:.1} h={:.1}",
            c.x,
            c.y,
            c.w,
            c.h,
            d.x,
            d.y,
            d.w,
            d.h,
        );
    }
}

// ── Clay helper ───────────────────────────────────────────────────────────

fn new_clay(size: f32) -> Clay {
    let mut clay = Clay::new((size, size).into());
    clay.set_measure_text_function_user_data((), clay_monospace_measure);
    clay
}

fn collect_clay_bboxes<'a>(
    commands: impl IntoIterator<Item = RenderCommand<'a, (), ()>>,
) -> Vec<Bbox> {
    let mut out = Vec::new();
    for cmd in commands {
        let kind = match cmd.config {
            RenderCommandConfig::Rectangle(_) => BboxKind::Rectangle,
            RenderCommandConfig::Text(_) => BboxKind::Text,
            _ => continue,
        };
        out.push(Bbox {
            x: cmd.bounding_box.x,
            y: cmd.bounding_box.y,
            w: cmd.bounding_box.width,
            h: cmd.bounding_box.height,
            kind,
        });
    }
    out
}

// ── Diegetic helper ───────────────────────────────────────────────────────

fn collect_diegetic_bboxes(result: &LayoutResult) -> Vec<Bbox> {
    let mut out = Vec::new();
    for cmd in &result.commands {
        let kind = match &cmd.kind {
            RenderCommandKind::Rectangle { .. } => BboxKind::Rectangle,
            RenderCommandKind::Text { .. } => BboxKind::Text,
            _ => continue,
        };
        out.push(Bbox {
            x: cmd.bounds.x,
            y: cmd.bounds.y,
            w: cmd.bounds.width,
            h: cmd.bounds.height,
            kind,
        });
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn parity_fixed_root_with_grow_child() {
    let size = 160.0;

    // Clay
    let mut clay = new_clay(size);
    let mut layout = clay.begin::<(), ()>();
    layout.with(
        Declaration::new()
            .layout()
            .width(fixed!(size))
            .height(fixed!(size))
            .end()
            .background_color((255, 0, 0).into()),
        |clay| {
            clay.with(
                Declaration::new()
                    .layout()
                    .width(grow!())
                    .height(grow!())
                    .end()
                    .background_color((0, 255, 0).into()),
                |_| {},
            );
        },
    );
    let clay_bboxes = collect_clay_bboxes(layout.end());

    // Diegetic
    let mut b = LayoutBuilder::with_root(
        El::new()
            .width(Sizing::fixed(size))
            .height(Sizing::fixed(size))
            .background(bevy::color::Color::srgb(1.0, 0.0, 0.0)),
    );
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(bevy::color::Color::srgb(0.0, 1.0, 0.0)),
        |_| {},
    );
    let tree = b.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, size, size, 1.0);
    let diegetic_bboxes = collect_diegetic_bboxes(&result);

    assert_bboxes_match(&clay_bboxes, &diegetic_bboxes, BboxKind::Rectangle);
}

#[test]
fn parity_header_body_divider() {
    let size = 160.0;

    // Clay
    let mut clay = new_clay(size);
    let mut layout = clay.begin::<(), ()>();
    layout.with(
        Declaration::new()
            .layout()
            .width(fixed!(size))
            .height(fixed!(size))
            .padding(clay_layout::layout::Padding::all(8))
            .direction(LayoutDirection::TopToBottom)
            .child_gap(5)
            .end()
            .background_color((180, 96, 122).into()),
        |clay| {
            // Header
            clay.with(
                Declaration::new()
                    .layout()
                    .width(grow!())
                    .height(fixed!(20.0))
                    .end()
                    .background_color((52, 98, 90).into()),
                |_| {},
            );
            // Divider
            clay.with(
                Declaration::new()
                    .layout()
                    .width(grow!())
                    .height(fixed!(4.0))
                    .end()
                    .background_color((74, 196, 172).into()),
                |_| {},
            );
            // Body
            clay.with(
                Declaration::new()
                    .layout()
                    .width(grow!())
                    .height(grow!())
                    .end()
                    .background_color((22, 28, 34).into()),
                |_| {},
            );
        },
    );
    let clay_bboxes = collect_clay_bboxes(layout.end());

    // Diegetic
    let mut b = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::fixed(size))
            .height(Sizing::fixed(size))
            .padding(Padding::all(8.0))
            .gap(5.0)
            .background(bevy::color::Color::srgb_u8(180, 96, 122)),
    );
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(20.0))
            .background(bevy::color::Color::srgb_u8(52, 98, 90)),
        |_| {},
    );
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(4.0))
            .background(bevy::color::Color::srgb_u8(74, 196, 172)),
        |_| {},
    );
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(bevy::color::Color::srgb_u8(22, 28, 34)),
        |_| {},
    );
    let tree = b.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, size, size, 1.0);
    let diegetic_bboxes = collect_diegetic_bboxes(&result);

    assert_bboxes_match(&clay_bboxes, &diegetic_bboxes, BboxKind::Rectangle);
}

#[test]
fn parity_key_value_row_with_spacer() {
    let size = 160.0;

    // Clay
    let mut clay = new_clay(size);
    let mut layout = clay.begin::<(), ()>();
    layout.with(
        Declaration::new()
            .layout()
            .width(fixed!(size))
            .height(fixed!(size))
            .direction(LayoutDirection::LeftToRight)
            .end()
            .background_color((22, 28, 34).into()),
        |clay| {
            clay.text(
                "fps:",
                clay_layout::text::TextConfig::new()
                    .font_size(CLAY_FONT_SIZE)
                    .end(),
            );
            clay.with(
                Declaration::new()
                    .layout()
                    .width(grow!())
                    .height(fixed!(1.0))
                    .end(),
                |_| {},
            );
            clay.text(
                "60",
                clay_layout::text::TextConfig::new()
                    .font_size(CLAY_FONT_SIZE)
                    .end(),
            );
        },
    );
    let clay_bboxes = collect_clay_bboxes(layout.end());

    // Diegetic
    let mut b = LayoutBuilder::with_root(
        El::row()
            .width(Sizing::fixed(size))
            .height(Sizing::fixed(size))
            .background(bevy::color::Color::srgb_u8(22, 28, 34)),
    );
    b.text(("fps:", TextStyle::new(FONT_SIZE)));
    b.with(
        El::new().width(Sizing::GROW).height(Sizing::fixed(1.0)),
        |_| {},
    );
    b.text(("60", TextStyle::new(FONT_SIZE)));
    let tree = b.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, size, size, 1.0);
    let diegetic_bboxes = collect_diegetic_bboxes(&result);

    assert_bboxes_match(&clay_bboxes, &diegetic_bboxes, BboxKind::Rectangle);
    assert_bboxes_match(&clay_bboxes, &diegetic_bboxes, BboxKind::Text);
}

#[test]
fn parity_vertical_center_alignment() {
    let size = 160.0;

    // Clay
    let mut clay = new_clay(size);
    let mut layout = clay.begin::<(), ()>();
    layout.with(
        Declaration::new()
            .layout()
            .width(fixed!(size))
            .height(fixed!(size))
            .direction(LayoutDirection::LeftToRight)
            .child_alignment(Alignment::new(
                LayoutAlignmentX::Left,
                LayoutAlignmentY::Center,
            ))
            .end()
            .background_color((52, 98, 90).into()),
        |clay| {
            clay.with(
                Declaration::new()
                    .layout()
                    .width(fixed!(40.0))
                    .height(fixed!(20.0))
                    .end()
                    .background_color((255, 0, 0).into()),
                |_| {},
            );
        },
    );
    let clay_bboxes = collect_clay_bboxes(layout.end());

    // Diegetic
    let mut b = LayoutBuilder::with_root(
        El::row()
            .width(Sizing::fixed(size))
            .height(Sizing::fixed(size))
            .align_y(AlignY::Center)
            .background(bevy::color::Color::srgb_u8(52, 98, 90)),
    );
    b.with(
        El::new()
            .width(Sizing::fixed(40.0))
            .height(Sizing::fixed(20.0))
            .background(bevy::color::Color::srgb(1.0, 0.0, 0.0)),
        |_| {},
    );
    let tree = b.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, size, size, 1.0);
    let diegetic_bboxes = collect_diegetic_bboxes(&result);

    assert_bboxes_match(&clay_bboxes, &diegetic_bboxes, BboxKind::Rectangle);
}

/// Builds the Clay layout for the fit-parent-with-grow-children centering test.
fn build_clay_fit_parent_centering(size: f32) -> Vec<Bbox> {
    let mut clay = new_clay(size);
    let mut layout = clay.begin::<(), ()>();
    layout.with(
        Declaration::new()
            .layout()
            .width(fixed!(size))
            .height(fixed!(size))
            .end(),
        |clay| {
            // Header container: fixed height, centers child vertically.
            clay.with(
                Declaration::new()
                    .layout()
                    .width(grow!())
                    .height(fixed!(30.0))
                    .padding(clay_layout::layout::Padding::new(0, 0, 4, 4))
                    .child_alignment(Alignment::new(
                        LayoutAlignmentX::Left,
                        LayoutAlignmentY::Center,
                    ))
                    .end()
                    .background_color((52, 98, 90).into()),
                |clay| {
                    // Text row: Fit height, LeftToRight.
                    clay.with(
                        Declaration::new()
                            .layout()
                            .width(grow!())
                            .height(fit!())
                            .direction(LayoutDirection::LeftToRight)
                            .end()
                            .background_color((22, 28, 34).into()),
                        |clay| {
                            clay.with(
                                Declaration::new()
                                    .layout()
                                    .width(fit!())
                                    .height(grow!())
                                    .end(),
                                |clay| {
                                    clay.text(
                                        "STATUS",
                                        clay_layout::text::TextConfig::new()
                                            .font_size(CLAY_FONT_SIZE)
                                            .end(),
                                    );
                                },
                            );
                            clay.with(
                                Declaration::new()
                                    .layout()
                                    .width(grow!())
                                    .height(fixed!(1.0))
                                    .end(),
                                |_| {},
                            );
                            clay.with(
                                Declaration::new()
                                    .layout()
                                    .width(fit!())
                                    .height(grow!())
                                    .end(),
                                |clay| {
                                    clay.text(
                                        "SUB",
                                        clay_layout::text::TextConfig::new()
                                            .font_size(CLAY_FONT_SIZE)
                                            .end(),
                                    );
                                },
                            );
                        },
                    );
                },
            );
        },
    );
    collect_clay_bboxes(layout.end())
}

/// Builds the diegetic layout for the fit-parent-with-grow-children centering test.
fn build_diegetic_fit_parent_centering(size: f32) -> Vec<Bbox> {
    let mut b = LayoutBuilder::with_root(
        El::new()
            .width(Sizing::fixed(size))
            .height(Sizing::fixed(size)),
    );
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(30.0))
            .padding(Padding::new(0.0, 0.0, 4.0, 4.0))
            .align_y(AlignY::Center)
            .background(bevy::color::Color::srgb_u8(52, 98, 90)),
        |b| {
            b.with(
                El::row()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .background(bevy::color::Color::srgb_u8(22, 28, 34)),
                |b| {
                    b.with(El::new().width(Sizing::FIT).height(Sizing::GROW), |b| {
                        b.text(("STATUS", TextStyle::new(FONT_SIZE)));
                    });
                    b.with(
                        El::new().width(Sizing::GROW).height(Sizing::fixed(1.0)),
                        |_| {},
                    );
                    b.with(El::new().width(Sizing::FIT).height(Sizing::GROW), |b| {
                        b.text(("SUB", TextStyle::new(FONT_SIZE)));
                    });
                },
            );
        },
    );
    let tree = b.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, size, size, 1.0);
    collect_diegetic_bboxes(&result)
}

#[test]
fn parity_fit_parent_with_grow_children_centering() {
    // The header vertical-centering bug: Fit-height parent, Grow-height
    // children containing text, centered vertically in a fixed container.
    let size = 160.0;

    let clay_bboxes = build_clay_fit_parent_centering(size);
    let diegetic_bboxes = build_diegetic_fit_parent_centering(size);

    assert_bboxes_match(&clay_bboxes, &diegetic_bboxes, BboxKind::Rectangle);
    assert_bboxes_match(&clay_bboxes, &diegetic_bboxes, BboxKind::Text);
}

#[test]
fn parity_compression_with_content_minimum() {
    // Two Fit siblings each with Fixed(50) content in an 80-wide parent.
    // Tests whether compression respects propagated content minimums.
    let size = 80.0;

    // Clay
    let mut clay = new_clay(size);
    let mut layout = clay.begin::<(), ()>();
    layout.with(
        Declaration::new()
            .layout()
            .width(fixed!(size))
            .height(fixed!(100.0))
            .direction(LayoutDirection::LeftToRight)
            .end()
            .background_color((22, 28, 34).into()),
        |clay| {
            clay.with(
                Declaration::new()
                    .layout()
                    .width(fit!())
                    .height(grow!())
                    .end()
                    .background_color((255, 0, 0).into()),
                |clay| {
                    clay.with(
                        Declaration::new()
                            .layout()
                            .width(fixed!(50.0))
                            .height(fixed!(10.0))
                            .end()
                            .background_color((0, 255, 0).into()),
                        |_| {},
                    );
                },
            );
            clay.with(
                Declaration::new()
                    .layout()
                    .width(fit!())
                    .height(grow!())
                    .end()
                    .background_color((0, 0, 255).into()),
                |clay| {
                    clay.with(
                        Declaration::new()
                            .layout()
                            .width(fixed!(50.0))
                            .height(fixed!(10.0))
                            .end()
                            .background_color((255, 255, 0).into()),
                        |_| {},
                    );
                },
            );
        },
    );
    let clay_bboxes = collect_clay_bboxes(layout.end());

    // Diegetic
    let mut b = LayoutBuilder::with_root(
        El::row()
            .width(Sizing::fixed(size))
            .height(Sizing::fixed(100.0))
            .background(bevy::color::Color::srgb_u8(22, 28, 34)),
    );
    b.with(
        El::new()
            .width(Sizing::FIT)
            .height(Sizing::GROW)
            .background(bevy::color::Color::srgb(1.0, 0.0, 0.0)),
        |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(50.0))
                    .height(Sizing::fixed(10.0))
                    .background(bevy::color::Color::srgb(0.0, 1.0, 0.0)),
                |_| {},
            );
        },
    );
    b.with(
        El::new()
            .width(Sizing::FIT)
            .height(Sizing::GROW)
            .background(bevy::color::Color::srgb(0.0, 0.0, 1.0)),
        |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(50.0))
                    .height(Sizing::fixed(10.0))
                    .background(bevy::color::Color::srgb(1.0, 1.0, 0.0)),
                |_| {},
            );
        },
    );
    let tree = b.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, size, 100.0, 1.0);
    let diegetic_bboxes = collect_diegetic_bboxes(&result);

    assert_bboxes_match(&clay_bboxes, &diegetic_bboxes, BboxKind::Rectangle);
}

#[test]
fn parity_cross_axis_grow_with_large_content() {
    // TopToBottom parent (30 wide), Grow child with Fixed(50) inner content.
    // Tests cross-axis minDimensions floor.
    let width = 30.0;
    let height = 100.0;

    // Clay
    let mut clay = Clay::new((width, height).into());
    clay.set_measure_text_function_user_data((), clay_monospace_measure);
    let mut layout = clay.begin::<(), ()>();
    layout.with(
        Declaration::new()
            .layout()
            .width(fixed!(width))
            .height(fixed!(height))
            .direction(LayoutDirection::TopToBottom)
            .end()
            .background_color((22, 28, 34).into()),
        |clay| {
            clay.with(
                Declaration::new()
                    .layout()
                    .width(grow!())
                    .height(grow!())
                    .end()
                    .background_color((255, 0, 0).into()),
                |clay| {
                    clay.with(
                        Declaration::new()
                            .layout()
                            .width(fixed!(50.0))
                            .height(fixed!(10.0))
                            .end()
                            .background_color((0, 255, 0).into()),
                        |_| {},
                    );
                },
            );
        },
    );
    let clay_bboxes = collect_clay_bboxes(layout.end());

    // Diegetic
    let mut b = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::fixed(width))
            .height(Sizing::fixed(height))
            .background(bevy::color::Color::srgb_u8(22, 28, 34)),
    );
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(bevy::color::Color::srgb(1.0, 0.0, 0.0)),
        |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(50.0))
                    .height(Sizing::fixed(10.0))
                    .background(bevy::color::Color::srgb(0.0, 1.0, 0.0)),
                |_| {},
            );
        },
    );
    let tree = b.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, width, height, 1.0);
    let diegetic_bboxes = collect_diegetic_bboxes(&result);

    assert_bboxes_match(&clay_bboxes, &diegetic_bboxes, BboxKind::Rectangle);
}

#[test]
fn parity_two_grow_children_horizontal() {
    let size = 200.0;

    // Clay
    let mut clay = new_clay(size);
    let mut layout = clay.begin::<(), ()>();
    layout.with(
        Declaration::new()
            .layout()
            .width(fixed!(size))
            .height(fixed!(100.0))
            .direction(LayoutDirection::LeftToRight)
            .end()
            .background_color((22, 28, 34).into()),
        |clay| {
            clay.with(
                Declaration::new()
                    .layout()
                    .width(grow!())
                    .height(grow!())
                    .end()
                    .background_color((255, 0, 0).into()),
                |_| {},
            );
            clay.with(
                Declaration::new()
                    .layout()
                    .width(grow!())
                    .height(grow!())
                    .end()
                    .background_color((0, 0, 255).into()),
                |_| {},
            );
        },
    );
    let clay_bboxes = collect_clay_bboxes(layout.end());

    // Diegetic
    let mut b = LayoutBuilder::with_root(
        El::row()
            .width(Sizing::fixed(size))
            .height(Sizing::fixed(100.0))
            .background(bevy::color::Color::srgb_u8(22, 28, 34)),
    );
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(bevy::color::Color::srgb(1.0, 0.0, 0.0)),
        |_| {},
    );
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(bevy::color::Color::srgb(0.0, 0.0, 1.0)),
        |_| {},
    );
    let tree = b.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, size, 100.0, 1.0);
    let diegetic_bboxes = collect_diegetic_bboxes(&result);

    assert_bboxes_match(&clay_bboxes, &diegetic_bboxes, BboxKind::Rectangle);
}

#[test]
fn parity_padding_and_child_gap() {
    let size = 160.0;

    // Clay
    let mut clay = new_clay(size);
    let mut layout = clay.begin::<(), ()>();
    layout.with(
        Declaration::new()
            .layout()
            .width(fixed!(size))
            .height(fixed!(size))
            .padding(clay_layout::layout::Padding::new(10, 20, 5, 15))
            .direction(LayoutDirection::TopToBottom)
            .child_gap(8)
            .end()
            .background_color((22, 28, 34).into()),
        |clay| {
            clay.with(
                Declaration::new()
                    .layout()
                    .width(grow!())
                    .height(fixed!(30.0))
                    .end()
                    .background_color((255, 0, 0).into()),
                |_| {},
            );
            clay.with(
                Declaration::new()
                    .layout()
                    .width(grow!())
                    .height(grow!())
                    .end()
                    .background_color((0, 255, 0).into()),
                |_| {},
            );
        },
    );
    let clay_bboxes = collect_clay_bboxes(layout.end());

    // Diegetic
    let mut b = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::fixed(size))
            .height(Sizing::fixed(size))
            .padding(Padding::new(10.0, 20.0, 5.0, 15.0))
            .gap(8.0)
            .background(bevy::color::Color::srgb_u8(22, 28, 34)),
    );
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(30.0))
            .background(bevy::color::Color::srgb(1.0, 0.0, 0.0)),
        |_| {},
    );
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(bevy::color::Color::srgb(0.0, 1.0, 0.0)),
        |_| {},
    );
    let tree = b.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, size, size, 1.0);
    let diegetic_bboxes = collect_diegetic_bboxes(&result);

    assert_bboxes_match(&clay_bboxes, &diegetic_bboxes, BboxKind::Rectangle);
}

#[test]
fn parity_right_alignment() {
    let size = 200.0;

    // Clay
    let mut clay = new_clay(size);
    let mut layout = clay.begin::<(), ()>();
    layout.with(
        Declaration::new()
            .layout()
            .width(fixed!(size))
            .height(fixed!(100.0))
            .direction(LayoutDirection::LeftToRight)
            .child_alignment(Alignment::new(
                LayoutAlignmentX::Right,
                LayoutAlignmentY::Top,
            ))
            .end()
            .background_color((22, 28, 34).into()),
        |clay| {
            clay.with(
                Declaration::new()
                    .layout()
                    .width(fixed!(50.0))
                    .height(fixed!(30.0))
                    .end()
                    .background_color((255, 0, 0).into()),
                |_| {},
            );
        },
    );
    let clay_bboxes = collect_clay_bboxes(layout.end());

    // Diegetic
    let mut b = LayoutBuilder::with_root(
        El::row()
            .width(Sizing::fixed(size))
            .height(Sizing::fixed(100.0))
            .align_x(AlignX::Right)
            .background(bevy::color::Color::srgb_u8(22, 28, 34)),
    );
    b.with(
        El::new()
            .width(Sizing::fixed(50.0))
            .height(Sizing::fixed(30.0))
            .background(bevy::color::Color::srgb(1.0, 0.0, 0.0)),
        |_| {},
    );
    let tree = b.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, size, 100.0, 1.0);
    let diegetic_bboxes = collect_diegetic_bboxes(&result);

    assert_bboxes_match(&clay_bboxes, &diegetic_bboxes, BboxKind::Rectangle);
}

/// Builds the Clay status panel layout with header, divider, and key-value body rows.
fn build_clay_status_panel(size: f32, labels: &[(&str, &str)]) -> Vec<Bbox> {
    let mut clay = new_clay(size);
    let mut layout = clay.begin::<(), ()>();
    layout.with(
        Declaration::new()
            .layout()
            .width(fixed!(size))
            .height(fixed!(size))
            .padding(clay_layout::layout::Padding::all(8))
            .direction(LayoutDirection::TopToBottom)
            .child_gap(5)
            .end()
            .background_color((180, 96, 122).into()),
        |clay| {
            build_clay_status_panel_header(clay);
            // Divider
            clay.with(
                Declaration::new()
                    .layout()
                    .width(grow!())
                    .height(fixed!(4.0))
                    .end()
                    .background_color((74, 196, 172).into()),
                |_| {},
            );
            build_clay_status_panel_body(clay, labels);
        },
    );
    collect_clay_bboxes(layout.end())
}

/// Builds the Clay header section: title row with "STATUS" / spacer / "DIEGETIC".
fn build_clay_status_panel_header<'a>(clay: &mut ClayLayoutScope<'a, 'a, (), ()>) {
    clay.with(
        Declaration::new()
            .layout()
            .width(grow!())
            .height(grow!(FONT_SIZE, 20.0))
            .padding(clay_layout::layout::Padding::new(5, 5, 4, 4))
            .child_alignment(Alignment::new(
                LayoutAlignmentX::Left,
                LayoutAlignmentY::Center,
            ))
            .end()
            .background_color((52, 98, 90).into()),
        |clay| {
            clay.with(
                Declaration::new()
                    .layout()
                    .width(grow!())
                    .height(fit!())
                    .direction(LayoutDirection::LeftToRight)
                    .end(),
                |clay| {
                    clay.with(
                        Declaration::new()
                            .layout()
                            .width(fit!())
                            .height(grow!())
                            .end(),
                        |clay| {
                            clay.text(
                                "STATUS",
                                clay_layout::text::TextConfig::new()
                                    .font_size(CLAY_FONT_SIZE)
                                    .end(),
                            );
                        },
                    );
                    clay.with(
                        Declaration::new()
                            .layout()
                            .width(grow!())
                            .height(fixed!(1.0))
                            .end(),
                        |_| {},
                    );
                    clay.with(
                        Declaration::new()
                            .layout()
                            .width(fit!())
                            .height(grow!())
                            .child_alignment(Alignment::new(
                                LayoutAlignmentX::Right,
                                LayoutAlignmentY::Top,
                            ))
                            .end(),
                        |clay| {
                            clay.text(
                                "DIEGETIC",
                                clay_layout::text::TextConfig::new()
                                    .font_size(CLAY_FONT_SIZE)
                                    .end(),
                            );
                        },
                    );
                },
            );
        },
    );
}

/// Builds the Clay body section: key-value rows inside a scrollable container.
fn build_clay_status_panel_body<'a>(
    clay: &mut ClayLayoutScope<'a, 'a, (), ()>,
    labels: &[(&str, &str)],
) {
    clay.with(
        Declaration::new()
            .layout()
            .width(grow!())
            .height(grow!())
            .end()
            .background_color((22, 28, 34).into()),
        |clay| {
            clay.with(
                Declaration::new()
                    .layout()
                    .width(grow!())
                    .padding(clay_layout::layout::Padding::all(5))
                    .direction(LayoutDirection::TopToBottom)
                    .child_gap(2)
                    .end(),
                |clay| {
                    for (label, value) in labels {
                        clay.with(
                            Declaration::new()
                                .layout()
                                .width(grow!())
                                .height(fit!())
                                .direction(LayoutDirection::LeftToRight)
                                .end(),
                            |clay| {
                                clay.text(
                                    label,
                                    clay_layout::text::TextConfig::new()
                                        .font_size(CLAY_FONT_SIZE)
                                        .end(),
                                );
                                clay.with(Declaration::new().layout().width(grow!()).end(), |_| {});
                                clay.text(
                                    value,
                                    clay_layout::text::TextConfig::new()
                                        .font_size(CLAY_FONT_SIZE)
                                        .end(),
                                );
                            },
                        );
                    }
                },
            );
        },
    );
}

/// Builds the diegetic status panel layout with header, divider, and key-value body rows.
fn build_diegetic_status_panel(size: f32, labels: &[(&str, &str)]) -> Vec<Bbox> {
    let mut b = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::fixed(size))
            .height(Sizing::fixed(size))
            .padding(Padding::all(8.0))
            .gap(5.0)
            .background(bevy::color::Color::srgb_u8(180, 96, 122)),
    );
    build_diegetic_status_panel_header(&mut b);
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(4.0))
            .background(bevy::color::Color::srgb_u8(74, 196, 172)),
        |_| {},
    );
    build_diegetic_status_panel_body(&mut b, labels);
    let tree = b.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, size, size, 1.0);
    collect_diegetic_bboxes(&result)
}

/// Builds the diegetic header section: title row with "STATUS" / spacer / "DIEGETIC".
fn build_diegetic_status_panel_header(b: &mut LayoutBuilder) {
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::grow_range(FONT_SIZE, 20.0))
            .padding(Padding::new(5.0, 5.0, 4.0, 4.0))
            .align_y(AlignY::Center)
            .background(bevy::color::Color::srgb_u8(52, 98, 90)),
        |b| {
            b.with(El::row().width(Sizing::GROW).height(Sizing::FIT), |b| {
                b.with(El::new().width(Sizing::FIT).height(Sizing::GROW), |b| {
                    b.text(("STATUS", TextStyle::new(FONT_SIZE)));
                });
                b.with(
                    El::new().width(Sizing::GROW).height(Sizing::fixed(1.0)),
                    |_| {},
                );
                b.with(
                    El::new()
                        .width(Sizing::FIT)
                        .height(Sizing::GROW)
                        .align_x(AlignX::Right),
                    |b| {
                        b.text(("DIEGETIC", TextStyle::new(FONT_SIZE)));
                    },
                );
            });
        },
    );
}

/// Builds the diegetic body section: key-value rows inside a scrollable container.
fn build_diegetic_status_panel_body(b: &mut LayoutBuilder, labels: &[(&str, &str)]) {
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(bevy::color::Color::srgb_u8(22, 28, 34)),
        |b| {
            b.with(
                El::column()
                    .width(Sizing::GROW)
                    .padding(Padding::all(5.0))
                    .gap(2.0),
                |b| {
                    for (label, value) in labels {
                        b.with(El::row().width(Sizing::GROW).height(Sizing::FIT), |b| {
                            b.text((*label, TextStyle::new(FONT_SIZE)));
                            b.with(
                                El::new().width(Sizing::GROW).height(Sizing::fixed(1.0)),
                                |_| {},
                            );
                            b.text((*value, TextStyle::new(FONT_SIZE)));
                        });
                    }
                },
            );
        },
    );
}

#[test]
fn parity_status_panel_full_layout() {
    // The full status panel layout from the actual application.
    let size = 160.0;
    let labels = [("fps:", "14"), ("frame ms:", "68"), ("radius:", "0.3")];

    let clay_bboxes = build_clay_status_panel(size, &labels);
    let diegetic_bboxes = build_diegetic_status_panel(size, &labels);

    assert_bboxes_match(&clay_bboxes, &diegetic_bboxes, BboxKind::Rectangle);
    assert_bboxes_match(&clay_bboxes, &diegetic_bboxes, BboxKind::Text);
}
