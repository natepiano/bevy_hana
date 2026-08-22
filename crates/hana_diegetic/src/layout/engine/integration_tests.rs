//! Layout engine integration tests.
//!
//! Each test constructs a layout tree, runs the engine, and verifies the
//! computed bounding boxes match expectations. A simple monospace text
//! measurement function is used throughout.

#![allow(
    clippy::float_cmp,
    reason = "tests compare exact expected layout values"
)]
#![allow(
    clippy::needless_collect,
    reason = "tests collect into named variables for readable assertions and index access"
)]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "tests use panicking helpers for clearer failure messages"
)]

use std::sync::Arc;

use bevy::color::Color;
use bevy_kana::ToF32;

use super::layout_engine::ComputedLayout;
use super::sizing;
use super::sizing::Axis;
use crate::constants::MONOSPACE_WIDTH_RATIO;
use crate::layout::AlignX;
use crate::layout::AlignY;
use crate::layout::Border;
use crate::layout::BoundingBox;
use crate::layout::ChildDivider;
use crate::layout::ChildLayoutState;
use crate::layout::Dimension;
use crate::layout::Direction;
use crate::layout::DrawOverflow;
use crate::layout::DrawZIndex;
use crate::layout::El;
use crate::layout::LayoutBuilder;
use crate::layout::LayoutEngine;
use crate::layout::LayoutTree;
use crate::layout::MeasureTextFn;
use crate::layout::Mm;
use crate::layout::Padding;
use crate::layout::PanelCoord;
use crate::layout::PanelDraw;
use crate::layout::PanelLine;
use crate::layout::PanelPoint;
use crate::layout::PanelShapeSourceKey;
use crate::layout::RectangleSource;
use crate::layout::RenderCommand;
use crate::layout::RenderCommandKind;
use crate::layout::ResolvedPanelShape;
use crate::layout::Sizing;
use crate::layout::Text;
use crate::layout::TextDimensions;
use crate::layout::TextMeasure;
use crate::layout::TextStyle;
use crate::layout::TextWrap;
use crate::layout::Unit;
use crate::layout::element::ElementContent;

const UNIT_GAP_CHILD_SIDE: f32 = 10.0;
const UNIT_GAP_LAYOUT_SCALE: f32 = 2.0;
const UNIT_GAP_MM: Mm = Mm(2.0);
const VIEWPORT: f32 = 200.0;

fn monospace_measure() -> MeasureTextFn {
    Arc::new(|text: &str, measure: &TextMeasure| {
        let char_width = measure.size * MONOSPACE_WIDTH_RATIO;
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

fn approx_eq(a: f32, b: f32) -> bool { (a - b).abs() < 0.01 }

fn text_width(text: &str, font_size: f32) -> f32 {
    text.chars().count().to_f32() * font_size * MONOSPACE_WIDTH_RATIO
}

fn line_height(font_size: f32) -> f32 { font_size }

fn text_height(line_count: u32, font_size: f32) -> f32 {
    line_count.to_f32() * line_height(font_size)
}

fn initialized_layouts(tree: &LayoutTree) -> Vec<ComputedLayout> {
    let measure = monospace_measure();
    let mut computed = vec![ComputedLayout::default(); tree.len()];
    for (index, element) in tree.elements.iter().enumerate() {
        computed[index].width = match element.width {
            Sizing::Fixed(size) => size.value,
            _ => 0.0,
        };
        computed[index].height = match element.height {
            Sizing::Fixed(size) => size.value,
            _ => 0.0,
        };

        if let ElementContent::Text { text, config, .. } = &element.content {
            let dims = measure(text, &config.as_measure().scaled(1.0));
            computed[index].natural_text_width = dims.width;
            if element.width.is_fit() {
                computed[index].width = dims
                    .width
                    .clamp(element.width.min_size(), element.width.max_size());
            }
            if element.height.is_fit() {
                computed[index].height = dims
                    .height
                    .clamp(element.height.min_size(), element.height.max_size());
            }
        }
    }
    computed
}

fn assert_layouts_match(actual: &[ComputedLayout], expected: &[ComputedLayout]) {
    assert_eq!(actual.len(), expected.len());
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert!(
            approx_eq(actual.width, expected.width),
            "width mismatch at {index}: {} != {}",
            actual.width,
            expected.width
        );
        assert!(
            approx_eq(actual.height, expected.height),
            "height mismatch at {index}: {} != {}",
            actual.height,
            expected.height
        );
        assert!(
            approx_eq(actual.min_width, expected.min_width),
            "min_width mismatch at {index}: {} != {}",
            actual.min_width,
            expected.min_width
        );
        assert!(
            approx_eq(actual.min_height, expected.min_height),
            "min_height mismatch at {index}: {} != {}",
            actual.min_height,
            expected.min_height
        );
    }
}

fn fixed_unit_gap_child() -> El {
    El::new()
        .width(Sizing::fixed(UNIT_GAP_CHILD_SIDE))
        .height(Sizing::fixed(UNIT_GAP_CHILD_SIDE))
}

fn scaled_unit_gap_tree(direction: Direction) -> LayoutTree {
    match direction {
        Direction::LeftToRight => scaled_unit_gap_tree_with_root(
            El::row()
                .width(Sizing::FIT)
                .height(Sizing::FIT)
                .gap(UNIT_GAP_MM),
        ),
        Direction::TopToBottom => scaled_unit_gap_tree_with_root(
            El::column()
                .width(Sizing::FIT)
                .height(Sizing::FIT)
                .gap(UNIT_GAP_MM),
        ),
    }
}

fn scaled_unit_gap_tree_with_root<L: ChildLayoutState>(root: El<L>) -> LayoutTree {
    let mut builder = LayoutBuilder::new(VIEWPORT, VIEWPORT);
    builder.with(root, |builder| {
        builder.with(fixed_unit_gap_child(), |_| {});
        builder.with(fixed_unit_gap_child(), |_| {});
    });
    builder
        .build()
        .scaled(UNIT_GAP_LAYOUT_SCALE, UNIT_GAP_LAYOUT_SCALE)
}

fn scaled_unit_gap() -> f32 { Dimension::from(UNIT_GAP_MM).to_points(UNIT_GAP_LAYOUT_SCALE) }

fn scaled_unit_gap_child_side() -> f32 { UNIT_GAP_CHILD_SIDE * UNIT_GAP_LAYOUT_SCALE }

fn line_commands(commands: &[RenderCommand]) -> Vec<&[ResolvedPanelShape]> {
    commands
        .iter()
        .filter_map(|command| match &command.kind {
            RenderCommandKind::PanelShapes { shapes } => Some(shapes.as_slice()),
            _ => None,
        })
        .collect()
}

fn command_index(
    commands: &[RenderCommand],
    predicate: impl Fn(&RenderCommandKind) -> bool,
) -> usize {
    commands
        .iter()
        .position(|command| predicate(&command.kind))
        .expect("command should exist")
}

#[test]
fn precompose_ldr_emits_single_boundary_command_without_descendants() {
    let mut builder = LayoutBuilder::new(100.0, 50.0);
    builder.with(
        El::column()
            .size(80.0, 30.0)
            .background(Color::WHITE)
            .precompose_ldr(),
        |builder| {
            builder.text(("child", TextStyle::new(10.0)));
        },
    );
    let tree = builder.build();
    let engine = LayoutEngine::new(monospace_measure());

    let result = engine.compute(&tree, 100.0, 50.0, 1.0);
    let precompose_count = result
        .commands
        .iter()
        .filter(|command| matches!(command.kind, RenderCommandKind::PrecomposeLdr))
        .count();

    assert_eq!(precompose_count, 1);
    assert!(
        !result
            .commands
            .iter()
            .any(|command| matches!(command.kind, RenderCommandKind::Text { .. }))
    );
    assert!(
        !result
            .commands
            .iter()
            .any(|command| matches!(command.kind, RenderCommandKind::Rectangle { .. }))
    );
}

fn add_aligned_table_row(
    builder: &mut LayoutBuilder,
    bindings: &[&str],
    action: &str,
    style: &TextStyle,
    action_min: f32,
) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(4.0)
            .align_y(AlignY::Center),
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .gap(2.0),
                |builder| {
                    for binding in bindings {
                        builder.text((*binding, style.clone()));
                    }
                },
            );
            builder.text(("->", style.clone()));
            builder.with(
                El::new()
                    .width(Sizing::fit_min(action_min))
                    .height(Sizing::FIT),
                |builder| {
                    builder.text((action, style.clone()));
                },
            );
        },
    );
}

// ── Fixed sizing ─────────────────────────────────────────────────────────────

#[test]
fn fixed_root_dimensions() {
    let tree = LayoutBuilder::new(100.0, 50.0).build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert_eq!(result.computed[0].width, 100.0);
    assert_eq!(result.computed[0].height, 50.0);
}

#[test]
fn fixed_child_dimensions() {
    let mut b = LayoutBuilder::new(200.0, 200.0);
    b.with(
        El::new()
            .width(Sizing::fixed(80.0))
            .height(Sizing::fixed(40.0)),
        |_| {},
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // Child is index 1.
    assert_eq!(result.computed[1].width, 80.0);
    assert_eq!(result.computed[1].height, 40.0);
}

#[test]
fn draw_commands_do_not_change_layout_bounds_or_non_line_commands() {
    let mut base_builder = LayoutBuilder::new(200.0, 200.0);
    base_builder.with(
        El::new()
            .width(Sizing::fixed(80.0))
            .height(Sizing::fixed(40.0)),
        |_| {},
    );
    let base_tree = base_builder.build();

    let mut draw_builder = LayoutBuilder::new(200.0, 200.0);
    draw_builder.with(
        El::new()
            .width(Sizing::fixed(80.0))
            .height(Sizing::fixed(40.0))
            .draw(PanelDraw::lines([PanelLine::new(
                PanelPoint::new(0.0, 0.0),
                PanelPoint::new(80.0, 40.0),
            )])),
        |_| {},
    );
    let draw_tree = draw_builder.build();

    let engine = LayoutEngine::new(monospace_measure());
    let base_result = engine.compute(&base_tree, VIEWPORT, VIEWPORT, 1.0);
    let draw_result = engine.compute(&draw_tree, VIEWPORT, VIEWPORT, 1.0);

    assert_eq!(
        draw_result.computed[1].bounds,
        base_result.computed[1].bounds
    );
    let draw_non_line_commands: Vec<_> = draw_result
        .commands
        .iter()
        .filter(|command| !matches!(command.kind, RenderCommandKind::PanelShapes { .. }))
        .collect();
    let base_non_line_commands: Vec<_> = base_result.commands.iter().collect();
    assert_eq!(draw_non_line_commands, base_non_line_commands);
    assert_eq!(line_commands(&draw_result.commands).len(), 1);
}

#[test]
fn line_commands_regenerate_from_cached_geometry() {
    let mut builder = LayoutBuilder::new(200.0, 200.0);
    builder.with(
        El::new()
            .width(Sizing::fixed(80.0))
            .height(Sizing::fixed(40.0))
            .draw(PanelDraw::lines([PanelLine::new((0.0, 0.0), (80.0, 40.0))])),
        |_| {},
    );
    let tree = builder.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);
    let mut regenerated = result.clone();

    regenerated.regenerate_commands(&tree);

    assert_eq!(regenerated.commands, result.commands);
    assert_eq!(line_commands(&regenerated.commands).len(), 1);
}

#[test]
fn line_commands_emit_before_child_text_and_shift_command_indices() {
    let text_style = TextStyle::new(12.0);
    let mut base_builder = LayoutBuilder::new(200.0, 100.0);
    base_builder.with(
        El::new()
            .width(Sizing::fixed(100.0))
            .height(Sizing::fixed(40.0)),
        |builder| {
            builder.text(("label", text_style.clone()));
        },
    );
    let base_tree = base_builder.build();

    let mut draw_builder = LayoutBuilder::new(200.0, 100.0);
    draw_builder.with(
        El::new()
            .width(Sizing::fixed(100.0))
            .height(Sizing::fixed(40.0))
            .draw(PanelDraw::lines([PanelLine::new(
                (0.0, 20.0),
                (100.0, 20.0),
            )])),
        |builder| {
            builder.text(("label", text_style));
        },
    );
    let draw_tree = draw_builder.build();

    let engine = LayoutEngine::new(monospace_measure());
    let base_result = engine.compute(&base_tree, VIEWPORT, VIEWPORT, 1.0);
    let draw_result = engine.compute(&draw_tree, VIEWPORT, VIEWPORT, 1.0);
    let base_text_index = command_index(&base_result.commands, |kind| {
        matches!(kind, RenderCommandKind::Text { .. })
    });
    let draw_text_index = command_index(&draw_result.commands, |kind| {
        matches!(kind, RenderCommandKind::Text { .. })
    });
    let line_index = command_index(&draw_result.commands, |kind| {
        matches!(kind, RenderCommandKind::PanelShapes { .. })
    });
    let lines = line_commands(&draw_result.commands);
    let resolved = &lines[0][0];

    assert!(line_index < draw_text_index);
    assert_eq!(draw_text_index, base_text_index + 1);
    assert_eq!(resolved.source_command_index, line_index);
}

#[test]
fn overflow_visible_line_clips_only_to_explicit_clipped_ancestor() {
    let mut builder = LayoutBuilder::new(100.0, 100.0);
    builder.with(
        El::new()
            .width(Sizing::fixed(50.0))
            .height(Sizing::fixed(50.0))
            .clip(),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::fixed(10.0))
                    .height(Sizing::fixed(10.0))
                    .draw(
                        PanelDraw::lines([PanelLine::new(
                            (0.0, 5.0),
                            (PanelCoord::end(-40.0), 5.0),
                        )])
                        .overflow(DrawOverflow::Visible),
                    ),
                |_| {},
            );
        },
    );
    let tree = builder.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);
    let line_index = command_index(&result.commands, |kind| {
        matches!(kind, RenderCommandKind::PanelShapes { .. })
    });
    let lines = line_commands(&result.commands);
    let resolved = &lines[0][0];

    assert_eq!(
        resolved.clip,
        Some(crate::layout::BoundingBox {
            x:      0.0,
            y:      0.0,
            width:  50.0,
            height: 50.0,
        })
    );
    assert_eq!(resolved.source_command_index, line_index);
    assert!(resolved.visual_bounds.x + resolved.visual_bounds.width > 10.0);
}

#[test]
fn overflow_visible_line_without_clipped_ancestor_escapes_viewport() {
    let mut builder = LayoutBuilder::new(100.0, 100.0);
    builder.with(
        El::new()
            .width(Sizing::fixed(50.0))
            .height(Sizing::fixed(50.0)),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::fixed(10.0))
                    .height(Sizing::fixed(10.0))
                    .draw(
                        PanelDraw::lines([PanelLine::new(
                            (0.0, 5.0),
                            (PanelCoord::end(-150.0), 5.0),
                        )])
                        .overflow(DrawOverflow::Visible),
                    ),
                |_| {},
            );
        },
    );
    let tree = builder.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);
    let lines = line_commands(&result.commands);
    let resolved = &lines[0][0];

    assert_eq!(resolved.clip, None);
    assert!(resolved.visual_bounds.x + resolved.visual_bounds.width > 100.0);
}

#[test]
fn clipped_line_uses_owner_bounds_inside_parent_clip() {
    let mut builder = LayoutBuilder::new(100.0, 100.0);
    builder.with(
        El::new()
            .width(Sizing::fixed(50.0))
            .height(Sizing::fixed(50.0))
            .clip(),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::fixed(10.0))
                    .height(Sizing::fixed(10.0))
                    .draw(PanelDraw::lines([PanelLine::new(
                        (0.0, 5.0),
                        (PanelCoord::end(-40.0), 5.0),
                    )])),
                |_| {},
            );
        },
    );
    let tree = builder.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);
    let lines = line_commands(&result.commands);
    let resolved = &lines[0][0];

    assert_eq!(
        resolved.clip,
        Some(crate::layout::BoundingBox {
            x:      0.0,
            y:      0.0,
            width:  10.0,
            height: 10.0,
        })
    );
}

#[test]
fn line_draw_regeneration_updates_visual_only_line_records() {
    let unchanged = PanelLine::new((0.0, 5.0), (40.0, 5.0));
    let second = PanelLine::new((0.0, 10.0), (40.0, 10.0));
    let mut original_builder = LayoutBuilder::new(100.0, 100.0);
    original_builder.with(
        El::new()
            .width(Sizing::fixed(40.0))
            .height(Sizing::fixed(20.0))
            .draw(PanelDraw::lines([unchanged.clone(), second])),
        |_| {},
    );
    let original_tree = original_builder.build();

    let mut updated_builder = LayoutBuilder::new(100.0, 100.0);
    updated_builder.with(
        El::new()
            .width(Sizing::fixed(40.0))
            .height(Sizing::fixed(20.0))
            .draw(PanelDraw::lines([
                unchanged.color(Color::srgb(1.0, 0.0, 0.0))
            ])),
        |_| {},
    );
    let updated_tree = updated_builder.build();

    let engine = LayoutEngine::new(monospace_measure());
    let mut result = engine.compute(&original_tree, VIEWPORT, VIEWPORT, 1.0);
    let original_key = line_commands(&result.commands)[0][0].source_key;

    result.regenerate_commands(&updated_tree);
    let updated_lines = line_commands(&result.commands);

    assert_eq!(updated_lines[0].len(), 1);
    assert_eq!(updated_lines[0][0].source_key, original_key);
    assert_eq!(updated_lines[0][0].color, Color::srgb(1.0, 0.0, 0.0));
}

#[test]
fn inserted_element_line_churns_later_ordinal_keys_without_stale_lines() {
    let first = PanelLine::new((0.0, 5.0), (40.0, 5.0));
    let second = PanelLine::new((0.0, 10.0), (40.0, 10.0));
    let inserted = PanelLine::new((0.0, 15.0), (40.0, 15.0));

    let mut original_builder = LayoutBuilder::new(100.0, 100.0);
    original_builder.with(
        El::new()
            .width(Sizing::fixed(40.0))
            .height(Sizing::fixed(20.0))
            .draw(PanelDraw::lines([first.clone(), second.clone()])),
        |_| {},
    );
    let original_tree = original_builder.build();

    let mut inserted_builder = LayoutBuilder::new(100.0, 100.0);
    inserted_builder.with(
        El::new()
            .width(Sizing::fixed(40.0))
            .height(Sizing::fixed(20.0))
            .draw(PanelDraw::lines([inserted, first, second])),
        |_| {},
    );
    let inserted_tree = inserted_builder.build();

    let engine = LayoutEngine::new(monospace_measure());
    let mut result = engine.compute(&original_tree, VIEWPORT, VIEWPORT, 1.0);
    let old_second_key = line_commands(&result.commands)[0][1].source_key;

    result.regenerate_commands(&inserted_tree);
    let updated_lines = line_commands(&result.commands);

    assert_eq!(updated_lines[0].len(), 3);
    assert_eq!(
        updated_lines[0][1].source_key,
        PanelShapeSourceKey::element(1, 0, 1)
    );
    assert_eq!(
        updated_lines[0][2].source_key,
        PanelShapeSourceKey::element(1, 0, 2)
    );
    assert_ne!(updated_lines[0][2].source_key, old_second_key);
}

#[test]
fn fused_fit_propagation_matches_separate_axis_passes() {
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::new(3.0, 4.0, 5.0, 6.0))
            .border(
                Border::new()
                    .left(1.0)
                    .right(2.0)
                    .top(3.0)
                    .bottom(4.0)
                    .color(Color::WHITE),
            )
            .gap(2.0),
    );
    builder.with(
        El::row().width(Sizing::FIT).height(Sizing::FIT).gap(4.0),
        |builder| {
            builder.text(("Alpha", TextStyle::new(10.0)));
            builder.with(
                El::new()
                    .width(Sizing::fixed(15.0))
                    .height(Sizing::fixed(7.0)),
                |_| {},
            );
        },
    );
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(12.0))
            .clip(),
        |builder| {
            builder.text(("Beta", TextStyle::new(8.0)));
        },
    );
    let tree = builder.build();

    let mut separate = initialized_layouts(&tree);
    sizing::propagate_fit_sizes(&tree, &mut separate, 0, Axis::X);
    sizing::propagate_fit_sizes(&tree, &mut separate, 0, Axis::Y);

    let mut fused = initialized_layouts(&tree);
    sizing::propagate_fit_sizes_xy(&tree, &mut fused, 0);

    assert_layouts_match(&fused, &separate);
}

// ── Grow sizing ──────────────────────────────────────────────────────────────

#[test]
fn single_grow_child_fills_parent() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert_eq!(result.computed[1].width, 200.0);
    assert_eq!(result.computed[1].height, 100.0);
}

#[test]
fn two_grow_children_split_evenly_horizontal() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(El::row().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
        b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // Parent is index 1, children are 2 and 3.
    assert!(approx_eq(result.computed[2].width, 100.0));
    assert!(approx_eq(result.computed[3].width, 100.0));
}

#[test]
fn two_grow_children_split_evenly_vertical() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(El::column().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
        b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[2].height, 50.0));
    assert!(approx_eq(result.computed[3].height, 50.0));
}

#[test]
fn grow_with_min_max() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(El::row().width(Sizing::GROW).height(Sizing::GROW), |b| {
        // This child wants to grow but is capped at 60.
        b.with(
            El::new()
                .width(Sizing::grow_range(0.0, 60.0))
                .height(Sizing::GROW),
            |_| {},
        );
        // This child fills the rest.
        b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[2].width, 60.0));
    assert!(approx_eq(result.computed[3].width, 140.0));
}

// ── Fit sizing ───────────────────────────────────────────────────────────────

#[test]
fn fit_wraps_text_content() {
    let mut b = LayoutBuilder::new(200.0, 200.0);
    let font_size = 16.0;
    let text = "Hello";
    // Expected text bounds come from the same monospace metrics as production.
    b.text((text, TextStyle::new(font_size)));
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(
        result.computed[1].width,
        text_width(text, font_size)
    ));
    assert!(approx_eq(result.computed[1].height, line_height(font_size)));
}

#[test]
fn fit_root_with_direct_text_child_resolves_width() {
    // Mirrors the WorldText / ScreenText one-element sugar: a Fit x Fit root
    // holding a single Fit text child directly (no intermediate El). The other
    // fit tests wrap text in a Grow/fixed El, so this direct-text-leaf case was
    // never exercised — it produced a zero-width root.
    let font_size = 16.0;
    let text = "Hello";
    let mut b = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    b.text((text, TextStyle::new(font_size)));
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(
        approx_eq(result.computed[0].width, text_width(text, font_size)),
        "root width = {}, expected {}",
        result.computed[0].width,
        text_width(text, font_size)
    );
    assert!(
        approx_eq(result.computed[0].height, line_height(font_size)),
        "root height = {}, expected {}",
        result.computed[0].height,
        line_height(font_size)
    );
}

#[test]
fn fit_max_saturates_instead_of_overflowing_when_scaled() {
    // The `Sizing::FIT` unbounded sentinel is `f32::MAX`. Scaling it by the
    // world meters->points factor (~2835x) must saturate at `f32::MAX`, not
    // overflow to `inf` — an `inf` max breaks Fit width resolution downstream.
    let scaled = Sizing::FIT.resolved(Unit::Meters.to_points());
    assert!(
        scaled.max_size().is_finite(),
        "scaled Fit max overflowed to {}",
        scaled.max_size()
    );
}

#[test]
fn fit_with_min_respects_minimum() {
    let mut b = LayoutBuilder::new(200.0, 200.0);
    b.with(
        El::new()
            .width(Sizing::fit_min(100.0))
            .height(Sizing::fixed(20.0)),
        |b| {
            // Content is only 48px wide but min is 100.
            b.text(("Hello", TextStyle::new(16.0)));
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // Parent element (index 1) should be at least 100 wide.
    assert!(result.computed[1].width >= 100.0);
}

// ── Engine sanity: Fit root with Grow children ──────────────────────────────

#[test]
fn fit_root_clamps_grow_children_content_under_max() {
    // Fit root (max 400) with two horizontal GROW children carrying text.
    // Expect the root to resolve to the combined text width, not the max.
    let font_size = 16.0;
    let text = "Hello";
    let expected_width = text_width(text, font_size) * 2.0;
    let mut b = LayoutBuilder::with_root(
        El::row()
            .width(Sizing::fit_range(0.0, 400.0))
            .height(Sizing::FIT),
    );
    b.with(El::new().width(Sizing::GROW).height(Sizing::FIT), |b| {
        b.text((text, TextStyle::new(font_size)));
    });
    b.with(El::new().width(Sizing::GROW).height(Sizing::FIT), |b| {
        b.text((text, TextStyle::new(font_size)));
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(
        approx_eq(result.computed[0].width, expected_width),
        "root width = {}, expected {expected_width}",
        result.computed[0].width
    );
}

#[test]
fn fit_root_caps_grow_children_content_at_max() {
    // Same tree, but combined text content is wider than max.
    let wide = "HelloHelloHelloHelloHelloHello";
    let font_size = 16.0;
    let expected_content_width = text_width(wide, font_size) * 2.0;
    let mut b = LayoutBuilder::with_root(
        El::row()
            .width(Sizing::fit_range(0.0, 400.0))
            .height(Sizing::FIT),
    );
    b.with(El::new().width(Sizing::GROW).height(Sizing::FIT), |b| {
        b.text((wide, TextStyle::new(font_size)));
    });
    b.with(El::new().width(Sizing::GROW).height(Sizing::FIT), |b| {
        b.text((wide, TextStyle::new(font_size)));
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(
        approx_eq(result.computed[0].width, 400.0),
        "root width = {}, expected 400 capped from content width {expected_content_width}",
        result.computed[0].width,
    );
}

// ── Percent sizing ───────────────────────────────────────────────────────────

#[test]
fn percent_sizing() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(El::row().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.with(
            El::new().width(Sizing::percent(0.3)).height(Sizing::GROW),
            |_| {},
        );
        b.with(
            El::new().width(Sizing::percent(0.7)).height(Sizing::GROW),
            |_| {},
        );
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[2].width, 60.0));
    assert!(approx_eq(result.computed[3].width, 140.0));
}

#[test]
fn row_main_percent_still_subtracts_chrome_and_gap() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(
        El::row()
            .width(Sizing::fixed(200.0))
            .height(Sizing::fixed(100.0))
            .padding(Padding::all(10.0))
            .border(Border::all(5.0, Color::WHITE))
            .gap(10.0),
        |b| {
            b.with(
                El::new().width(Sizing::percent(0.5)).height(Sizing::GROW),
                |_| {},
            );
            b.with(
                El::new().width(Sizing::percent(0.5)).height(Sizing::GROW),
                |_| {},
            );
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[2].width, 80.0));
    assert!(approx_eq(result.computed[3].width, 80.0));
    assert!(approx_eq(result.computed[2].height, 70.0));
}

#[test]
fn row_cross_percent_keeps_existing_parent_size_basis() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(
        El::row()
            .width(Sizing::fixed(200.0))
            .height(Sizing::fixed(100.0))
            .padding(Padding::all(10.0))
            .border(Border::all(5.0, Color::WHITE)),
        |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(20.0))
                    .height(Sizing::percent(0.5)),
                |_| {},
            );
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[2].height, 50.0));
}

#[test]
fn column_main_grow_still_splits_content_after_chrome_and_gap() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(
        El::column()
            .width(Sizing::fixed(200.0))
            .height(Sizing::fixed(100.0))
            .padding(Padding::all(10.0))
            .border(Border::all(5.0, Color::WHITE))
            .gap(10.0),
        |b| {
            b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
            b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[2].height, 30.0));
    assert!(approx_eq(result.computed[3].height, 30.0));
    assert!(approx_eq(result.computed[2].width, 170.0));
}

// ── Overlay layout ───────────────────────────────────────────────────────────

fn overlay_child_bounds(align_x: AlignX, align_y: AlignY) -> BoundingBox {
    let mut b = LayoutBuilder::with_root(
        El::overlay()
            .width(Sizing::fixed(100.0))
            .height(Sizing::fixed(80.0))
            .alignment(align_x, align_y),
    );
    b.with(
        El::new()
            .width(Sizing::fixed(20.0))
            .height(Sizing::fixed(10.0)),
        |_| {},
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);
    result.computed[1].bounds
}

#[test]
fn overlay_fit_uses_max_child_extent_and_chrome() {
    let mut b = LayoutBuilder::with_root(
        El::overlay()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::new(3.0, 5.0, 7.0, 11.0))
            .border(
                Border::new()
                    .left(2.0)
                    .right(4.0)
                    .top(6.0)
                    .bottom(8.0)
                    .color(Color::WHITE),
            ),
    );
    b.with(
        El::new()
            .width(Sizing::fixed(20.0))
            .height(Sizing::fixed(30.0)),
        |_| {},
    );
    b.with(
        El::new()
            .width(Sizing::fixed(50.0))
            .height(Sizing::fixed(10.0)),
        |_| {},
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[0].width, 64.0));
    assert!(approx_eq(result.computed[0].height, 62.0));
}

#[test]
fn overlay_top_left_alignment_positions_children_at_content_origin() {
    let bounds = overlay_child_bounds(AlignX::Left, AlignY::Top);

    assert!(approx_eq(bounds.x, 0.0));
    assert!(approx_eq(bounds.y, 0.0));
}

#[test]
fn overlay_center_alignment_positions_children_in_both_axes() {
    let bounds = overlay_child_bounds(AlignX::Center, AlignY::Center);

    assert!(approx_eq(bounds.x, 40.0));
    assert!(approx_eq(bounds.y, 35.0));
}

#[test]
fn overlay_bottom_right_alignment_positions_children_in_both_axes() {
    let bounds = overlay_child_bounds(AlignX::Right, AlignY::Bottom);

    assert!(approx_eq(bounds.x, 80.0));
    assert!(approx_eq(bounds.y, 70.0));
}

#[test]
fn overlay_positioning_uses_padding_and_border_offsets() {
    let mut b = LayoutBuilder::with_root(
        El::overlay()
            .width(Sizing::fixed(100.0))
            .height(Sizing::fixed(80.0))
            .padding(Padding::new(3.0, 5.0, 7.0, 11.0))
            .border(Border::new().left(2.0).top(6.0).color(Color::WHITE)),
    );
    b.with(
        El::new()
            .width(Sizing::fixed(20.0))
            .height(Sizing::fixed(10.0)),
        |_| {},
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[1].bounds.x, 5.0));
    assert!(approx_eq(result.computed[1].bounds.y, 13.0));
}

#[test]
fn overlay_percent_and_grow_size_against_content_box() {
    let mut b = LayoutBuilder::with_root(
        El::overlay()
            .width(Sizing::fixed(200.0))
            .height(Sizing::fixed(100.0)),
    );
    b.with(
        El::new().width(Sizing::GROW).height(Sizing::percent(0.5)),
        |_| {},
    );
    b.with(
        El::new().width(Sizing::percent(0.25)).height(Sizing::GROW),
        |_| {},
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[1].width, 200.0));
    assert!(approx_eq(result.computed[1].height, 50.0));
    assert!(approx_eq(result.computed[2].width, 50.0));
    assert!(approx_eq(result.computed[2].height, 100.0));
}

#[test]
fn overlay_scroll_extents_are_independent_per_axis() {
    let mut b = LayoutBuilder::with_root(
        El::overlay()
            .width(Sizing::fixed(100.0))
            .height(Sizing::fixed(80.0))
            .scroll_x(70.0)
            .scroll_y(50.0),
    );
    b.with(
        El::new()
            .width(Sizing::fixed(150.0))
            .height(Sizing::fixed(120.0)),
        |_| {},
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[1].bounds.x, -50.0));
    assert!(approx_eq(result.computed[1].bounds.y, -40.0));
}

#[test]
fn scroll_y_from_end_does_not_change_horizontal_anchor() {
    let mut b = LayoutBuilder::with_root(
        El::overlay()
            .width(Sizing::fixed(100.0))
            .height(Sizing::fixed(80.0))
            .scroll_x(10.0)
            .scroll_y_from_end(0.0),
    );
    b.with(
        El::new()
            .width(Sizing::fixed(150.0))
            .height(Sizing::fixed(120.0)),
        |_| {},
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[1].bounds.x, -10.0));
    assert!(approx_eq(result.computed[1].bounds.y, -40.0));
}

#[test]
fn bordered_overlay_text_wraps_inside_content_box() {
    let font_size = 10.0;
    let mut b = LayoutBuilder::with_root(
        El::overlay()
            .width(Sizing::fixed(80.0))
            .height(Sizing::fixed(100.0))
            .border(Border::all(10.0, Color::WHITE)),
    );
    b.text(Text::new("Hello World", TextStyle::new(font_size)).wrap(TextWrap::Words));
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(
        result.computed[1].height,
        text_height(2, font_size)
    ));
}

#[test]
fn overlay_does_not_emit_between_child_dividers() {
    let mut b = LayoutBuilder::with_root(
        El::overlay()
            .width(Sizing::fixed(100.0))
            .height(Sizing::fixed(80.0)),
    );
    b.with(
        El::new()
            .width(Sizing::fixed(20.0))
            .height(Sizing::fixed(20.0)),
        |_| {},
    );
    b.with(
        El::new()
            .width(Sizing::fixed(20.0))
            .height(Sizing::fixed(20.0)),
        |_| {},
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);
    let divider_count = result
        .commands
        .iter()
        .filter(|command| {
            matches!(
                command.kind,
                RenderCommandKind::Rectangle {
                    source: RectangleSource::ChildDivider { .. },
                    ..
                }
            )
        })
        .count();

    assert_eq!(divider_count, 0);
}

#[test]
fn overlay_overlapped_children_preserve_draw_z_index() {
    const LOWERED_LEVEL: DrawZIndex = DrawZIndex(-1);
    const RAISED_LEVEL: DrawZIndex = DrawZIndex(1);

    let mut b = LayoutBuilder::with_root(
        El::overlay()
            .width(Sizing::fixed(100.0))
            .height(Sizing::fixed(80.0)),
    );
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(Color::BLACK)
            .z_index(LOWERED_LEVEL),
        |_| {},
    );
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(Color::WHITE)
            .z_index(RAISED_LEVEL),
        |_| {},
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);
    let background_commands = result
        .commands
        .iter()
        .filter(|command| {
            matches!(
                command.kind,
                RenderCommandKind::Rectangle {
                    source: RectangleSource::Background,
                    ..
                }
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(background_commands.len(), 2);
    assert_eq!(background_commands[0].bounds, background_commands[1].bounds);
    assert_eq!(background_commands[0].z_index, LOWERED_LEVEL);
    assert_eq!(background_commands[1].z_index, RAISED_LEVEL);
}

// ── Padding ──────────────────────────────────────────────────────────────────

#[test]
fn padding_reduces_child_space() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(10.0)),
        |b| {
            b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // Parent is 200x100 with 10px padding on each side.
    // Child should be 180x80.
    assert!(approx_eq(result.computed[2].width, 180.0));
    assert!(approx_eq(result.computed[2].height, 80.0));
}

#[test]
fn asymmetric_padding() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::new(5.0, 15.0, 10.0, 20.0)),
        |b| {
            b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // Horizontal padding: 5 + 15 = 20, vertical: 10 + 20 = 30.
    assert!(approx_eq(result.computed[2].width, 180.0));
    assert!(approx_eq(result.computed[2].height, 70.0));
}

// ── Child gap ────────────────────────────────────────────────────────────────

#[test]
fn child_gap_horizontal() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(
        El::row().width(Sizing::GROW).height(Sizing::GROW).gap(10.0),
        |b| {
            b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
            b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
            b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // 200 - 2*10 gap = 180 / 3 = 60 each.
    assert!(approx_eq(result.computed[2].width, 60.0));
    assert!(approx_eq(result.computed[3].width, 60.0));
    assert!(approx_eq(result.computed[4].width, 60.0));
}

#[test]
fn child_gap_vertical() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(5.0),
        |b| {
            b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
            b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // 100 - 5 gap = 95 / 2 = 47.5 each.
    assert!(approx_eq(result.computed[2].height, 47.5));
    assert!(approx_eq(result.computed[3].height, 47.5));
}

#[test]
fn row_unit_backed_gap_scales() {
    let tree = scaled_unit_gap_tree(Direction::LeftToRight);

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(
        &tree,
        VIEWPORT * UNIT_GAP_LAYOUT_SCALE,
        VIEWPORT * UNIT_GAP_LAYOUT_SCALE,
        1.0,
    );

    let child_side = scaled_unit_gap_child_side();
    let expected_second_x = child_side + scaled_unit_gap();
    assert!(approx_eq(result.computed[3].bounds.x, expected_second_x));
    assert!(approx_eq(
        result.computed[1].width,
        expected_second_x + child_side
    ));
}

#[test]
fn column_unit_backed_gap_scales() {
    let tree = scaled_unit_gap_tree(Direction::TopToBottom);

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(
        &tree,
        VIEWPORT * UNIT_GAP_LAYOUT_SCALE,
        VIEWPORT * UNIT_GAP_LAYOUT_SCALE,
        1.0,
    );

    let child_side = scaled_unit_gap_child_side();
    let expected_second_y = child_side + scaled_unit_gap();
    assert!(approx_eq(result.computed[3].bounds.y, expected_second_y));
    assert!(approx_eq(
        result.computed[1].height,
        expected_second_y + child_side
    ));
}

// ── Alignment ────────────────────────────────────────────────────────────────

#[test]
fn center_alignment_horizontal() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .align_x(AlignX::Center),
        |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(50.0))
                    .height(Sizing::fixed(30.0)),
                |_| {},
            );
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // Find the child's render command to check position.
    // Extra space = 200 - 50 = 150. Center offset = 75.
    let child_bounds = result.computed[2].bounds;
    assert!(approx_eq(child_bounds.x, 75.0));
}

#[test]
fn right_alignment_horizontal() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .align_x(AlignX::Right),
        |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(50.0))
                    .height(Sizing::fixed(30.0)),
                |_| {},
            );
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    let child_bounds = result.computed[2].bounds;
    assert!(approx_eq(child_bounds.x, 150.0));
}

#[test]
fn center_alignment_vertical() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .align_y(AlignY::Center),
        |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(50.0))
                    .height(Sizing::fixed(30.0)),
                |_| {},
            );
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // Cross-axis centering: (100 - 30) / 2 = 35.
    let child_bounds = result.computed[2].bounds;
    assert!(approx_eq(child_bounds.y, 35.0));
}

#[test]
fn bottom_alignment_vertical() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .align_y(AlignY::Bottom),
        |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(50.0))
                    .height(Sizing::fixed(30.0)),
                |_| {},
            );
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // Bottom: (100 - 30) = 70.
    let child_bounds = result.computed[2].bounds;
    assert!(approx_eq(child_bounds.y, 70.0));
}

// ── Direction ────────────────────────────────────────────────────────────────

#[test]
fn left_to_right_positioning() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(El::row().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.with(
            El::new()
                .width(Sizing::fixed(60.0))
                .height(Sizing::fixed(40.0)),
            |_| {},
        );
        b.with(
            El::new()
                .width(Sizing::fixed(80.0))
                .height(Sizing::fixed(40.0)),
            |_| {},
        );
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    let first = result.computed[2].bounds;
    let second = result.computed[3].bounds;
    assert!(approx_eq(first.x, 0.0));
    assert!(approx_eq(second.x, 60.0));
}

#[test]
fn top_to_bottom_positioning() {
    let mut b = LayoutBuilder::new(200.0, 200.0);
    b.with(El::column().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.with(
            El::new()
                .width(Sizing::fixed(60.0))
                .height(Sizing::fixed(30.0)),
            |_| {},
        );
        b.with(
            El::new()
                .width(Sizing::fixed(60.0))
                .height(Sizing::fixed(50.0)),
            |_| {},
        );
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    let first = result.computed[2].bounds;
    let second = result.computed[3].bounds;
    assert!(approx_eq(first.y, 0.0));
    assert!(approx_eq(second.y, 30.0));
}

// ── Scroll offset ────────────────────────────────────────────────────────────

/// Builds a 200×100 clipping column of four 50px-tall children (content 200,
/// viewport 100, so max scroll = 100) scrolled vertically by `offset`. The
/// scroll container is index 1; the children are indices 2..=5.
fn scroll_column(offset: f32) -> super::LayoutResult {
    let mut b = LayoutBuilder::new(200.0, 120.0);
    b.with(
        El::column()
            .width(Sizing::fixed(200.0))
            .height(Sizing::fixed(100.0))
            .scroll_y(offset),
        |b| {
            for _ in 0..4 {
                b.with(
                    El::new().width(Sizing::GROW).height(Sizing::fixed(50.0)),
                    |_| {},
                );
            }
        },
    );
    let tree = b.build();
    LayoutEngine::new(monospace_measure()).compute(&tree, VIEWPORT, VIEWPORT, 1.0)
}

#[test]
fn scroll_y_shifts_children_up_by_offset() {
    let result = scroll_column(30.0);

    // First child slides up by the offset; the rest follow at a 50px stride.
    assert!(approx_eq(result.computed[2].bounds.y, -30.0));
    assert!(approx_eq(result.computed[5].bounds.y, 120.0));
}

#[test]
fn scroll_y_clamps_to_bottom_with_max() {
    let result = scroll_column(f32::MAX);

    // Clamped to max scroll (content 200 − viewport 100 = 100): the last child's
    // bottom edge lands on the viewport bottom (−100 + 3·50 + 50 = 100).
    assert!(approx_eq(result.computed[2].bounds.y, -100.0));
    assert!(approx_eq(result.computed[5].bounds.y, 50.0));
}

#[test]
fn clipped_container_fills_parent_not_content() {
    // A clipping GROW column inside a fixed 100px-tall root must fill the root
    // (100), not inflate to its 200px of children — otherwise it overflows every
    // ancestor instead of clipping/scrolling its own content.
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(
        El::column().width(Sizing::GROW).height(Sizing::GROW).clip(),
        |b| {
            for _ in 0..4 {
                b.with(
                    El::new().width(Sizing::GROW).height(Sizing::fixed(50.0)),
                    |_| {},
                );
            }
        },
    );
    let tree = b.build();
    let result = LayoutEngine::new(monospace_measure()).compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[1].height, 100.0));
}

#[test]
fn scroll_y_from_end_pins_to_bottom_at_zero() {
    // A four-child column (content 200, viewport 100): scrollback 0 shows the
    // bottom, walking the last child's bottom edge to the viewport bottom.
    let mut b = LayoutBuilder::new(200.0, 120.0);
    b.with(
        El::column()
            .width(Sizing::fixed(200.0))
            .height(Sizing::fixed(100.0))
            .scroll_y_from_end(0.0),
        |b| {
            for _ in 0..4 {
                b.with(
                    El::new().width(Sizing::GROW).height(Sizing::fixed(50.0)),
                    |_| {},
                );
            }
        },
    );
    let tree = b.build();
    let result = LayoutEngine::new(monospace_measure()).compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // Same as scroll_y(f32::MAX): pinned to the bottom.
    assert!(approx_eq(result.computed[2].bounds.y, -100.0));
    assert!(approx_eq(result.computed[5].bounds.y, 50.0));
}

#[test]
fn scroll_y_from_end_walks_upward() {
    // scrollback 40 from a max of 100 leaves an effective offset of 60.
    let mut b = LayoutBuilder::new(200.0, 120.0);
    b.with(
        El::column()
            .width(Sizing::fixed(200.0))
            .height(Sizing::fixed(100.0))
            .scroll_y_from_end(40.0),
        |b| {
            for _ in 0..4 {
                b.with(
                    El::new().width(Sizing::GROW).height(Sizing::fixed(50.0)),
                    |_| {},
                );
            }
        },
    );
    let tree = b.build();
    let result = LayoutEngine::new(monospace_measure()).compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[2].bounds.y, -60.0));
}

#[test]
fn scroll_y_clamps_to_zero_when_content_fits() {
    // One 50px child in a 100px viewport: nothing to scroll, offset clamps to 0.
    let mut b = LayoutBuilder::new(200.0, 120.0);
    b.with(
        El::column()
            .width(Sizing::fixed(200.0))
            .height(Sizing::fixed(100.0))
            .scroll_y(40.0),
        |b| {
            b.with(
                El::new().width(Sizing::GROW).height(Sizing::fixed(50.0)),
                |_| {},
            );
        },
    );
    let tree = b.build();
    let result = LayoutEngine::new(monospace_measure()).compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[2].bounds.y, 0.0));
}

// ── Overflow compression ─────────────────────────────────────────────────────

#[test]
fn overflow_compression_largest_first() {
    let mut b = LayoutBuilder::new(100.0, 50.0);
    b.with(El::row().width(Sizing::GROW).height(Sizing::GROW), |b| {
        // Total: 60 + 60 = 120, but parent is 100 wide.
        // Both are Fit with Fixed(60) content. Clay's `minDimensions`
        // propagates 60 from each child, so neither can compress below 60.
        // Result: both stay at 60, overflowing the parent by 20.
        b.with(
            El::new()
                .width(Sizing::fit_range(0.0, 60.0))
                .height(Sizing::GROW),
            |b| {
                b.with(
                    El::new()
                        .width(Sizing::fixed(60.0))
                        .height(Sizing::fixed(10.0)),
                    |_| {},
                );
            },
        );
        b.with(
            El::new()
                .width(Sizing::fit_range(0.0, 60.0))
                .height(Sizing::GROW),
            |b| {
                b.with(
                    El::new()
                        .width(Sizing::fixed(60.0))
                        .height(Sizing::fixed(10.0)),
                    |_| {},
                );
            },
        );
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // `minDimensions` floor prevents compression below content size.
    assert!(approx_eq(result.computed[2].width, 60.0));
    assert!(approx_eq(result.computed[4].width, 60.0));
}

// ── Render commands ──────────────────────────────────────────────────────────

#[test]
fn render_commands_include_rectangles() {
    let mut b = LayoutBuilder::new(100.0, 100.0);
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(Color::srgb_u8(255, 0, 0)),
        |_| {},
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    let rect_commands: Vec<_> = result
        .commands
        .iter()
        .filter(|cmd| matches!(cmd.kind, RenderCommandKind::Rectangle { .. }))
        .collect();

    assert!(
        !rect_commands.is_empty(),
        "Should have at least one rectangle render command"
    );
}

#[test]
fn empty_draw_sibling_does_not_hide_fixed_background_sibling() {
    const RULER_WIDTH: f32 = 10.0;
    const RULER_HEIGHT: f32 = 297.0;
    const TICK_TRACK_WIDTH: f32 = 5.0;
    const SPINE_WIDTH: f32 = 0.2;

    let mut builder = LayoutBuilder::new(RULER_WIDTH, RULER_HEIGHT);
    builder.with(
        El::row().width(Sizing::GROW).height(Sizing::GROW),
        |builder| {
            builder.with(
                El::column().width(Sizing::GROW).height(Sizing::GROW),
                |builder| {
                    // Ruler numerals overflow their narrow label column instead
                    // of reserving min-content width, so opt out of the
                    // `TextWrap::Words` default.
                    builder.text(
                        Text::new("29", TextStyle::new(8.0).with_color(Color::WHITE))
                            .wrap(TextWrap::None),
                    );
                },
            );
            builder.with(
                El::row()
                    .width(Sizing::fixed(TICK_TRACK_WIDTH + SPINE_WIDTH))
                    .height(Sizing::fixed(RULER_HEIGHT)),
                |builder| {
                    builder.with(
                        El::new()
                            .width(Sizing::GROW)
                            .height(Sizing::GROW)
                            .draw(PanelDraw::lines([PanelLine::new(
                                PanelPoint::new(0.0, 0.5),
                                PanelPoint::new(TICK_TRACK_WIDTH, 0.5),
                            )
                            .width(0.3)
                            .color(Color::WHITE)])),
                        |_| {},
                    );
                    builder.with(
                        El::new()
                            .width(Sizing::fixed(SPINE_WIDTH))
                            .height(Sizing::GROW)
                            .background(Color::WHITE),
                        |_| {},
                    );
                },
            );
        },
    );

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&builder.build(), RULER_WIDTH, RULER_HEIGHT, 1.0);
    let spine = result
        .commands
        .iter()
        .find(|command| {
            matches!(command.kind, RenderCommandKind::Rectangle { .. })
                && approx_eq(command.bounds.width, SPINE_WIDTH)
                && approx_eq(command.bounds.height, RULER_HEIGHT)
        })
        .expect("spine rectangle should be emitted");
    let line_count = result
        .commands
        .iter()
        .filter_map(|command| match &command.kind {
            RenderCommandKind::PanelShapes { shapes } => Some(shapes.len()),
            _ => None,
        })
        .sum::<usize>();

    assert!(approx_eq(spine.bounds.x, RULER_WIDTH - SPINE_WIDTH));
    assert_eq!(line_count, 1);
}

#[test]
fn render_commands_include_text() {
    let mut b = LayoutBuilder::new(200.0, 200.0);
    b.text(("Hello", TextStyle::new(16.0)));
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    let text_commands: Vec<_> = result
        .commands
        .iter()
        .filter(|cmd| matches!(cmd.kind, RenderCommandKind::Text { .. }))
        .collect();

    assert_eq!(text_commands.len(), 1);
    if let RenderCommandKind::Text { ref text, .. } = text_commands[0].kind {
        assert_eq!(text, "Hello");
    }
}

#[test]
fn render_commands_include_borders() {
    let mut b = LayoutBuilder::new(100.0, 100.0);
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .border(Border::all(2.0, Color::srgb_u8(255, 255, 255))),
        |_| {},
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    let border_commands: Vec<_> = result
        .commands
        .iter()
        .filter(|cmd| matches!(cmd.kind, RenderCommandKind::Border { .. }))
        .collect();

    assert!(
        !border_commands.is_empty(),
        "Should have at least one border render command"
    );
}

// ── Nested layout ────────────────────────────────────────────────────────────

#[test]
fn nested_layout_header_body() {
    // Mimics the status cube layout: header + divider + body.
    let mut b = LayoutBuilder::new(160.0, 160.0);
    b.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(8.0))
            .gap(5.0),
        |b| {
            // Header: fixed height.
            b.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(20.0))
                    .background(Color::srgb_u8(52, 98, 90)),
                |b| {
                    b.text(("STATUS", TextStyle::new(7.0)));
                },
            );
            // Divider.
            b.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(4.0))
                    .background(Color::srgb_u8(74, 196, 172)),
                |_| {},
            );
            // Body: grows to fill remaining space.
            b.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .background(Color::srgb_u8(22, 28, 34)),
                |_| {},
            );
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // Root (0): 160x160.
    // Container (1): 160x160 (Grow fills root, which has no padding).
    // Container's children get 160 - 2*8 padding = 144 available.
    let container = &result.computed[1];
    assert!(approx_eq(container.width, 160.0));
    assert!(approx_eq(container.height, 160.0));

    // Header (2): 144 wide (grows within container's padding), 20 tall (fixed).
    let header = &result.computed[2];
    assert!(approx_eq(header.width, 144.0));
    assert!(approx_eq(header.height, 20.0));

    // Divider (4): 144 wide, 4 tall.
    let divider = &result.computed[4];
    assert!(approx_eq(divider.width, 144.0));
    assert!(approx_eq(divider.height, 4.0));

    // Body (5): 144 wide, fills rest. 144 - 20 - 4 - 2*5 gap = 110.
    let body = &result.computed[5];
    assert!(approx_eq(body.width, 144.0));
    assert!(approx_eq(body.height, 110.0));
}

// ── Positioning with padding ─────────────────────────────────────────────────

#[test]
fn children_positioned_after_padding() {
    let mut b = LayoutBuilder::new(200.0, 200.0);
    b.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::new(20.0, 10.0, 30.0, 5.0)),
        |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(50.0))
                    .height(Sizing::fixed(50.0)),
                |_| {},
            );
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // Child should start at (20, 30) — left padding, top padding.
    let child_bounds = result.computed[2].bounds;
    assert!(approx_eq(child_bounds.x, 20.0));
    assert!(approx_eq(child_bounds.y, 30.0));
}

// ── Mixed fixed and grow ─────────────────────────────────────────────────────

#[test]
fn fixed_and_grow_siblings() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(El::row().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.with(
            El::new().width(Sizing::fixed(50.0)).height(Sizing::GROW),
            |_| {},
        );
        b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(approx_eq(result.computed[2].width, 50.0));
    assert!(approx_eq(result.computed[3].width, 150.0));
}

// ── Text positioning ─────────────────────────────────────────────────────────

#[test]
fn text_positioned_correctly() {
    let mut b = LayoutBuilder::new(200.0, 200.0);
    b.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(10.0)),
        |b| {
            b.text(("Hello", TextStyle::new(16.0)));
            b.text(("World", TextStyle::new(16.0)));
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // "Hello" at (10, 10), "World" at (10, 26).
    let hello = result.computed[2].bounds;
    let world = result.computed[3].bounds;
    assert!(approx_eq(hello.x, 10.0));
    assert!(approx_eq(hello.y, 10.0));
    assert!(approx_eq(world.x, 10.0));
    assert!(approx_eq(world.y, 26.0));
}

// ── Key-value row layout ─────────────────────────────────────────────────────

#[test]
fn key_value_row_layout() {
    // Label on left, value pushed to right by grow spacer.
    let label_text = "fps:";
    let value_text = "60";
    let font_size = 7.0;
    let label_width = text_width(label_text, font_size);
    let value_width = text_width(value_text, font_size);
    let spacer_width = VIEWPORT - label_width - value_width;
    let mut b = LayoutBuilder::new(200.0, 200.0);
    b.with(El::row().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.text((label_text, TextStyle::new(font_size)));
        b.with(
            El::new().width(Sizing::GROW).height(Sizing::fixed(1.0)),
            |_| {},
        );
        b.text((value_text, TextStyle::new(font_size)));
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // Spacer fills the space left after the label and value text widths.
    let label = result.computed[2].bounds;
    let value = result.computed[4].bounds;
    assert!(approx_eq(label.x, 0.0));
    assert!(approx_eq(result.computed[3].bounds.width, spacer_width));
    assert!(approx_eq(value.x, VIEWPORT - value_width));
}

#[test]
fn fit_table_with_grow_rows_aligns_middle_column() {
    let font_size = 10.0;
    let style = TextStyle::new(font_size);
    let action_min = text_width("Orbit", font_size);
    let mut b = LayoutBuilder::new(200.0, 200.0);
    b.with(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(5.0)
            .child_divider(ChildDivider::new(1.0, Color::WHITE)),
        |builder| {
            add_aligned_table_row(
                builder,
                &["MMB drag", "Trackpad"],
                "Orbit",
                &style,
                action_min,
            );
            add_aligned_table_row(
                builder,
                &["Shift+MMB drag", "Shift+trackpad"],
                "Pan",
                &style,
                action_min,
            );
            add_aligned_table_row(
                builder,
                &["Wheel", "Ctrl+trackpad", "Pinch"],
                "Zoom",
                &style,
                action_min,
            );
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    let arrow_xs = result
        .commands
        .iter()
        .filter_map(|command| match &command.kind {
            RenderCommandKind::Text { text, .. } if text == "->" => Some(command.bounds.x),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(arrow_xs.len(), 3);
    assert!(approx_eq(arrow_xs[0], arrow_xs[1]));
    assert!(approx_eq(arrow_xs[1], arrow_xs[2]));

    let divider_count = result
        .commands
        .iter()
        .filter(|command| {
            matches!(
                command.kind,
                RenderCommandKind::Rectangle {
                    source: RectangleSource::ChildDivider { .. },
                    ..
                }
            )
        })
        .count();
    assert_eq!(divider_count, 2);
}

// ── Scissor/clip ─────────────────────────────────────────────────────────────

#[test]
fn clip_emits_scissor_commands() {
    let mut b = LayoutBuilder::new(100.0, 100.0);
    b.with(
        El::new().width(Sizing::GROW).height(Sizing::GROW).clip(),
        |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(50.0))
                    .height(Sizing::fixed(50.0)),
                |_| {},
            );
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    let scissor_starts: Vec<_> = result
        .commands
        .iter()
        .filter(|cmd| matches!(cmd.kind, RenderCommandKind::ScissorStart))
        .collect();
    let scissor_ends: Vec<_> = result
        .commands
        .iter()
        .filter(|cmd| matches!(cmd.kind, RenderCommandKind::ScissorEnd))
        .collect();

    assert_eq!(scissor_starts.len(), 1);
    assert_eq!(scissor_ends.len(), 1);
}

// ── Empty tree ───────────────────────────────────────────────────────────────

#[test]
fn empty_tree_produces_no_commands() {
    let tree = LayoutTree::new();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    assert!(result.commands.is_empty());
    assert!(result.computed.is_empty());
}

// ── Child dividers ───────────────────────────────────────────────────────────

#[test]
fn child_dividers_emitted() {
    let mut b = LayoutBuilder::new(200.0, 100.0);
    b.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .child_divider(ChildDivider::new(2.0, Color::srgb_u8(255, 255, 255))),
        |b| {
            b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
            b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
            b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // Three children produce two child-divider rectangles.
    let rect_commands: Vec<_> = result
        .commands
        .iter()
        .filter(|cmd| {
            matches!(
                cmd.kind,
                RenderCommandKind::Rectangle {
                    source: RectangleSource::ChildDivider { .. },
                    ..
                }
            ) && cmd.element_idx == 1
        })
        .collect();

    assert_eq!(rect_commands.len(), 2);
}

#[test]
fn cached_geometry_regeneration_preserves_up_traversal_commands() {
    let mut builder = LayoutBuilder::new(120.0, 80.0);
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .border(Border::all(2.0, Color::WHITE))
            .child_divider(ChildDivider::new(1.0, Color::BLACK))
            .clip(),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(20.0))
                    .background(Color::srgb(0.1, 0.2, 0.3)),
                |_| {},
            );
            builder.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(20.0))
                    .background(Color::srgb(0.3, 0.2, 0.1)),
                |_| {},
            );
        },
    );
    let tree = builder.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);
    let mut regenerated = result.clone();

    regenerated.regenerate_commands(&tree);

    assert_eq!(regenerated.commands, result.commands);
    let border_count = regenerated
        .commands
        .iter()
        .filter(|command| command.element_idx == 1)
        .filter(|command| matches!(command.kind, RenderCommandKind::Border { .. }))
        .count();
    let divider_count = regenerated
        .commands
        .iter()
        .filter(|command| command.element_idx == 1)
        .filter(|command| {
            matches!(
                command.kind,
                RenderCommandKind::Rectangle {
                    source: RectangleSource::ChildDivider { .. },
                    ..
                }
            )
        })
        .count();
    let scissor_start_count = regenerated
        .commands
        .iter()
        .filter(|command| command.element_idx == 1)
        .filter(|command| matches!(command.kind, RenderCommandKind::ScissorStart))
        .count();
    let scissor_end_count = regenerated
        .commands
        .iter()
        .filter(|command| command.element_idx == 1)
        .filter(|command| matches!(command.kind, RenderCommandKind::ScissorEnd))
        .count();

    assert_eq!(border_count, 1);
    assert_eq!(divider_count, 1);
    assert_eq!(scissor_start_count, 1);
    assert_eq!(scissor_end_count, 1);
}

// ── Text wrapping ─────────────────────────────────────────────────────────────

#[test]
fn text_wraps_at_word_boundaries() {
    let font_size = 16.0;
    let hello_width = text_width("Hello", font_size);
    let world_width = text_width("World", font_size);
    let test_width = text_width("Test", font_size);
    let space_width = text_width(" ", font_size);
    let first_line_width = hello_width + space_width + world_width;
    // "Hello World Test" is wider than the container before wrapping.
    // Container is 80 wide — should force word wrapping.
    // "Hello World" is wider than the container, so the three words split
    // across separate lines.
    assert!(first_line_width > 80.0);
    assert!(test_width < 80.0);
    let mut b = LayoutBuilder::new(80.0, 200.0);
    b.with(El::column().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.text(Text::new("Hello World Test", TextStyle::new(font_size)).wrap(TextWrap::Words));
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 80.0, 200.0, 1.0);

    // Text element is index 2. Height should be three measured lines.
    assert!(approx_eq(
        result.computed[2].height,
        text_height(3, font_size)
    ));

    // Should emit 3 text render commands for the wrapped lines.
    let text_commands: Vec<_> = result
        .commands
        .iter()
        .filter(|cmd| matches!(cmd.kind, RenderCommandKind::Text { .. }))
        .collect();
    assert_eq!(text_commands.len(), 3, "Should have 3 wrapped text lines");
}

#[test]
fn text_no_wrap_overflows() {
    // "Hello World" in a narrow container with TextWrap::None.
    let mut b = LayoutBuilder::new(40.0, 200.0);
    b.with(El::column().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.text(Text::new("Hello World", TextStyle::new(16.0)).wrap(TextWrap::None));
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 40.0, 200.0, 1.0);

    // Height should remain single-line (16.0).
    assert!(approx_eq(result.computed[2].height, 16.0));

    // Single text command emitted (no wrapping).
    let text_commands: Vec<_> = result
        .commands
        .iter()
        .filter(|cmd| matches!(cmd.kind, RenderCommandKind::Text { .. }))
        .collect();
    assert_eq!(text_commands.len(), 1);
}

#[test]
fn fit_chain_wraps_words_text_at_ancestor_bound() {
    let font_size = 16.0;
    // Bound fits "Alpha Beta" (10 chars) but not "Alpha Beta Gamma" (16 chars).
    let bound = text_width("Alpha Betax", font_size);
    let mut b = LayoutBuilder::new(500.0, 200.0);
    b.with(
        El::column().width(Sizing::fixed(bound)).height(Sizing::FIT),
        |b| {
            b.with(El::column().width(Sizing::FIT).height(Sizing::FIT), |b| {
                b.text(
                    Text::new("Alpha Beta Gamma", TextStyle::new(font_size)).wrap(TextWrap::Words),
                );
            });
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 500.0, 200.0, 1.0);

    // Element 2 is the Fit column: clamped to the fixed ancestor's content
    // size instead of keeping the natural text width.
    assert!(approx_eq(result.computed[2].width, bound));
    // Element 3 is the text leaf: wrapped to two lines within the bound.
    assert!(approx_eq(
        result.computed[3].width,
        text_width("Alpha Beta", font_size)
    ));
    assert!(approx_eq(
        result.computed[3].height,
        text_height(2, font_size)
    ));
    let text_commands = result
        .commands
        .iter()
        .filter(|cmd| matches!(cmd.kind, RenderCommandKind::Text { .. }))
        .count();
    assert_eq!(text_commands, 2);
}

#[test]
fn fit_words_text_floors_at_widest_word() {
    let font_size = 16.0;
    let widest_word_width = text_width("Alpha", font_size);
    // Bound narrower than the widest word: the Fit chain floors at the word.
    let bound = text_width("Alph", font_size);
    let mut b = LayoutBuilder::new(500.0, 200.0);
    b.with(
        El::column().width(Sizing::fixed(bound)).height(Sizing::FIT),
        |b| {
            b.with(El::column().width(Sizing::FIT).height(Sizing::FIT), |b| {
                b.text(
                    Text::new("Alpha Beta Gamma", TextStyle::new(font_size)).wrap(TextWrap::Words),
                );
            });
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 500.0, 200.0, 1.0);

    // The text leaf's propagated minimum is its widest word.
    assert!(approx_eq(result.computed[3].min_width, widest_word_width));
    // Both the Fit column and the text floor at the widest word, overflowing
    // the narrower fixed ancestor instead of breaking inside a word.
    assert!(approx_eq(result.computed[2].width, widest_word_width));
    assert!(approx_eq(result.computed[3].width, widest_word_width));
    // One word per line.
    assert!(approx_eq(
        result.computed[3].height,
        text_height(3, font_size)
    ));
}

#[test]
fn unbounded_fit_words_text_stays_single_line() {
    let font_size = 16.0;
    let natural_width = text_width("Alpha Beta Gamma", font_size);
    let mut b = LayoutBuilder::new(500.0, 200.0);
    b.with(El::column().width(Sizing::FIT).height(Sizing::FIT), |b| {
        b.text(Text::new("Alpha Beta Gamma", TextStyle::new(font_size)).wrap(TextWrap::Words));
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 500.0, 200.0, 1.0);

    // No bound anywhere in the chain: the text keeps its natural width.
    assert!(approx_eq(result.computed[2].width, natural_width));
    assert!(approx_eq(result.computed[2].height, line_height(font_size)));
    let text_commands = result
        .commands
        .iter()
        .filter(|cmd| matches!(cmd.kind, RenderCommandKind::Text { .. }))
        .count();
    assert_eq!(text_commands, 1);
}

#[test]
fn text_measure_as_reserves_surrogate_size() {
    let font_size = 16.0;
    let mut b = LayoutBuilder::new(500.0, 200.0);
    b.text(Text::new("OK", TextStyle::new(font_size)).measure_as("Disconnected"));
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 500.0, 200.0, 1.0);

    assert!(approx_eq(
        result.computed[1].width,
        text_width("Disconnected", font_size)
    ));
    assert!(approx_eq(result.computed[1].height, line_height(font_size)));

    let text_commands: Vec<_> = result
        .commands
        .iter()
        .filter_map(|cmd| match &cmd.kind {
            RenderCommandKind::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text_commands, vec!["OK"]);
}

#[test]
fn text_wrap_at_measure_reserves_surrogate_width() {
    let font_size = 16.0;
    let mut b = LayoutBuilder::new(500.0, 200.0);
    b.text(Text::new("Alpha Beta Gamma", TextStyle::new(font_size)).wrap_at_measure("Alpha Beta"));
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 500.0, 200.0, 1.0);

    assert!(approx_eq(
        result.computed[1].width,
        text_width("Alpha Beta", font_size)
    ));
    assert!(approx_eq(
        result.computed[1].height,
        text_height(2, font_size)
    ));

    let text_commands: Vec<_> = result
        .commands
        .iter()
        .filter_map(|cmd| match &cmd.kind {
            RenderCommandKind::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text_commands, vec!["Alpha Beta", "Gamma"]);
}

#[test]
fn text_wraps_at_newlines_only() {
    // "Line1\nLine2\nLine3" with TextWrap::Newlines in a wide container.
    let font_size = 16.0;
    let mut b = LayoutBuilder::new(500.0, 200.0);
    b.with(El::column().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.text(
            Text::new("Line1\nLine2\nLine3", TextStyle::new(font_size)).wrap(TextWrap::Newlines),
        );
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 500.0, 200.0, 1.0);

    assert!(approx_eq(
        result.computed[2].height,
        text_height(3, font_size)
    ));

    let text_commands: Vec<_> = result
        .commands
        .iter()
        .filter(|cmd| matches!(cmd.kind, RenderCommandKind::Text { .. }))
        .collect();
    assert_eq!(text_commands.len(), 3);
}

#[test]
fn word_wrap_long_word_does_not_break() {
    let font_size = 16.0;
    let word = "Supercalifragilistic";
    let word_width = text_width(word, font_size);
    // Container is only 80 wide. The word should NOT be broken — stays on one line.
    assert!(word_width > 80.0);
    let mut b = LayoutBuilder::new(80.0, 200.0);
    b.with(El::column().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.text(Text::new(word, TextStyle::new(font_size)).wrap(TextWrap::Words));
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 80.0, 200.0, 1.0);

    // Should be a single line (word never broken mid-word).
    assert!(approx_eq(result.computed[2].height, line_height(font_size)));

    let text_commands: Vec<_> = result
        .commands
        .iter()
        .filter(|cmd| matches!(cmd.kind, RenderCommandKind::Text { .. }))
        .collect();
    assert_eq!(text_commands.len(), 1);
}

#[test]
fn word_wrap_preserves_explicit_newlines() {
    let font_size = 16.0;
    let first_paragraph_width = text_width("AA BB", font_size);
    let second_paragraph_width = text_width("CC DD", font_size);
    // "AA BB\nCC DD" with Words wrap in a container that fits "AA BB" but not all four.
    // Container is 50 wide — "AA BB" fits on one line, "CC DD" fits on one line.
    assert!(first_paragraph_width < 50.0);
    assert!(second_paragraph_width < 50.0);
    let mut b = LayoutBuilder::new(50.0, 200.0);
    b.with(El::column().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.text(Text::new("AA BB\nCC DD", TextStyle::new(font_size)).wrap(TextWrap::Words));
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 50.0, 200.0, 1.0);

    // Two paragraphs, each fits on one line.
    assert!(approx_eq(
        result.computed[2].height,
        text_height(2, font_size)
    ));

    let text_commands: Vec<_> = result
        .commands
        .iter()
        .filter(|cmd| matches!(cmd.kind, RenderCommandKind::Text { .. }))
        .collect();
    assert_eq!(text_commands.len(), 2);
}

#[test]
fn word_wrap_empty_string() {
    let font_size = 16.0;
    let mut b = LayoutBuilder::new(200.0, 200.0);
    b.with(El::column().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.text(Text::new("", TextStyle::new(font_size)).wrap(TextWrap::Words));
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 200.0, 200.0, 1.0);

    // Empty text produces one empty line.
    assert!(approx_eq(result.computed[2].height, line_height(font_size)));
}

#[test]
fn word_wrap_updates_parent_fit_height() {
    let font_size = 16.0;
    // Parent is Fit-height, child text wraps to 3 lines.
    // Parent height should grow to accommodate.
    let mut b = LayoutBuilder::new(80.0, 200.0);
    b.with(El::column().width(Sizing::GROW).height(Sizing::FIT), |b| {
        b.text(Text::new("Hello World Test", TextStyle::new(font_size)).wrap(TextWrap::Words));
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 80.0, 200.0, 1.0);

    // Text wraps to 3 lines (see text_wraps_at_word_boundaries test).
    // Parent Fit height should follow the measured wrapped text height.
    assert!(approx_eq(
        result.computed[1].height,
        text_height(3, font_size)
    ));
}

#[test]
fn word_wrap_render_commands_per_line() {
    // Verify each wrapped line has correct bounds positioning.
    let font_size = 16.0;
    let first_line_width = text_width("AA BB", font_size);
    let second_line_width = text_width("CC", font_size);
    let mut b = LayoutBuilder::new(50.0, 200.0);
    b.with(El::column().width(Sizing::GROW).height(Sizing::GROW), |b| {
        // The first wrapped line fits in the container; the second starts
        // one measured line height lower.
        assert!(first_line_width < 50.0);
        assert!(second_line_width < 50.0);
        b.text(Text::new("AA BB CC", TextStyle::new(font_size)).wrap(TextWrap::Words));
    });
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 50.0, 200.0, 1.0);

    let text_commands: Vec<_> = result
        .commands
        .iter()
        .filter(|cmd| matches!(cmd.kind, RenderCommandKind::Text { .. }))
        .collect();

    assert_eq!(text_commands.len(), 2);

    // First line starts at y=0 (relative to parent).
    assert!(approx_eq(text_commands[0].bounds.y, 0.0));

    // Second line starts one measured line height down.
    assert!(approx_eq(text_commands[1].bounds.y, line_height(font_size)));
}

// ── Fit parent with Grow children propagation ─────────────────────────────

#[test]
fn fit_parent_sees_grow_children_content_height() {
    let title_font_size = 7.0;
    let subtitle_font_size = 4.0;
    let text_row_height = line_height(title_font_size);
    // Reproduces the header vertical-centering bug: a Fit-height parent
    // with Grow-height children that contain text. The Fit parent must
    // propagate the children's content size upward so it gets a real
    // height, not collapse to the spacer's 1.0.
    //
    // Layout (mirrors the status panel header):
    //   header_container (Grow height, padding 4/4, align_y=Center)
    //     text_row (Fit height, LeftToRight)
    //       title_slot (Fit width, Grow height) → text "STATUS" (font 7)
    //       spacer (Grow width, Fixed height=1)
    //       subtitle_slot (Fit width, Grow height) → text "SUB" (font 4)
    //
    // text_row Fit height should be the title line height, NOT the spacer.
    let mut b = LayoutBuilder::new(160.0, 160.0);
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(20.0))
            .padding(Padding::new(0.0, 0.0, 4.0, 4.0))
            .align_y(AlignY::Center),
        |b| {
            b.with(El::row().width(Sizing::GROW).height(Sizing::FIT), |b| {
                b.with(El::new().width(Sizing::FIT).height(Sizing::GROW), |b| {
                    b.text(("STATUS", TextStyle::new(title_font_size)));
                });
                b.with(
                    El::new().width(Sizing::GROW).height(Sizing::fixed(1.0)),
                    |_| {},
                );
                b.with(El::new().width(Sizing::FIT).height(Sizing::GROW), |b| {
                    b.text(("SUB", TextStyle::new(subtitle_font_size)));
                });
            });
        },
    );
    let tree = b.build();

    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 160.0, 160.0, 1.0);

    // text_row is index 2.
    let computed_text_row_height = result.computed[2].height;
    assert!(
        computed_text_row_height >= text_row_height - 0.01,
        "Fit text_row height should be >= {text_row_height} (text content), got {computed_text_row_height}"
    );

    // text_row should be vertically centered in header_container.
    // header_container is 20.0 tall with 4+4=8 vertical padding → 12 content area.
    // Center offset = (12 - text_row_height) / 2 + 4 (top padding).
    let text_row_bounds = result.computed[2].bounds;
    let expected_y = (12.0 - computed_text_row_height).mul_add(0.5, 4.0);
    assert!(
        approx_eq(text_row_bounds.y, expected_y),
        "text_row should be centered: expected y={expected_y}, got y={}",
        text_row_bounds.y
    );
}

// ── Clay parity: minDimensions propagation ────────────────────────────────
//
// Clay tracks a propagated `minDimensions` field on every element — the
// recursive minimum size derived from nested content. Our engine does not
// track this yet. These tests encode Clay's correct behavior and will fail
// until we implement `minDimensions`.

#[test]
fn compression_respects_content_minimum_symmetric() {
    // Two Fit siblings each containing a Fixed(50) child in an 80-wide parent.
    // Total content = 100, overflow = 20.
    //
    // Clay: `minDimensions` = 50 for each Fit wrapper (propagated from the
    // Fixed child). Compression cannot reduce either below 50. Both stay at
    // 50 — the parent overflows by 20.
    //
    // Our engine: `min_size()` = 0 (default Fit), so compression squashes
    // both to 40.
    let mut b = LayoutBuilder::new(80.0, 100.0);
    b.with(El::row().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.with(El::new().width(Sizing::FIT).height(Sizing::GROW), |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(50.0))
                    .height(Sizing::fixed(10.0)),
                |_| {},
            );
        });
        b.with(El::new().width(Sizing::FIT).height(Sizing::GROW), |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(50.0))
                    .height(Sizing::fixed(10.0)),
                |_| {},
            );
        });
    });
    let tree = b.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 80.0, 100.0, 1.0);

    // Indices: 0=root, 1=container, 2=fit_a, 3=fixed_a, 4=fit_b, 5=fixed_b
    let child_a = result.computed[2].width;
    let child_b = result.computed[4].width;
    assert!(
        child_a >= 50.0 - 0.01,
        "Fit child A should not compress below content minimum 50.0, got {child_a}"
    );
    assert!(
        child_b >= 50.0 - 0.01,
        "Fit child B should not compress below content minimum 50.0, got {child_b}"
    );
}

#[test]
fn compression_respects_content_minimum_asymmetric() {
    // Fit child A has Fixed(60) content, Fit child B has Fixed(30). Parent is 80.
    // Total = 90, overflow = 10.
    //
    // Clay: A has `minDimensions` = 60, B has `minDimensions` = 30. Compression
    // targets the largest (A at 60) but `minDimensions` prevents any reduction.
    // Both stay at their content size.
    //
    // Our engine: compresses A from 60 to 50 (largest-first, 10px distributed).
    let mut b = LayoutBuilder::new(80.0, 100.0);
    b.with(El::row().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.with(El::new().width(Sizing::FIT).height(Sizing::GROW), |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(60.0))
                    .height(Sizing::fixed(10.0)),
                |_| {},
            );
        });
        b.with(El::new().width(Sizing::FIT).height(Sizing::GROW), |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(30.0))
                    .height(Sizing::fixed(10.0)),
                |_| {},
            );
        });
    });
    let tree = b.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 80.0, 100.0, 1.0);

    let child_a = result.computed[2].width;
    assert!(
        child_a >= 60.0 - 0.01,
        "Fit child A should not compress below content minimum 60.0, got {child_a}"
    );
}

#[test]
fn cross_axis_grow_respects_content_minimum() {
    // Clay's cross-axis sizing enforces `minDimensions` as a floor.
    // A Grow child whose nested content is wider than the parent should
    // not be squished below its content minimum.
    //
    // Layout: TopToBottom parent (30 wide), Grow-width child containing
    // a Fixed(50) inner element.
    //
    // Clay: cross-axis sets Grow to `MIN(parent-padding, max)` = 30, then
    // applies `MAX(minDimensions=50, 30)` = 50. Content minimum wins.
    //
    // Our engine: Grow fills parent = 30. No content floor.
    let mut b = LayoutBuilder::new(30.0, 100.0);
    b.with(El::column().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |b| {
            b.with(
                El::new()
                    .width(Sizing::fixed(50.0))
                    .height(Sizing::fixed(10.0)),
                |_| {},
            );
        });
    });
    let tree = b.build();
    let engine = LayoutEngine::new(monospace_measure());
    let result = engine.compute(&tree, 30.0, 100.0, 1.0);

    // Index 2 is the Grow child.
    let child_width = result.computed[2].width;
    assert!(
        child_width >= 50.0 - 0.01,
        "Cross-axis Grow child should not shrink below content minimum 50.0, got {child_width}"
    );
}

#[test]
fn grow_body_compression_20_rows() {
    // Reproduces the benchmark parity failure: root 160x160, TopToBottom,
    // with header(Grow 10..20), divider(Fixed 4), body(Grow).
    // 20 rows of text overflow the available space. Clay keeps body at its
    // content height (248), header at content height (18). We should match.
    let size = 160.0_f32;
    let measure = monospace_measure();

    let rows: Vec<(&str, &str)> = (0..20)
        .map(|i| {
            let labels = ["fps:", "frame ms:", "radius:", "entities:", "triangles:"];
            let values = ["60", "16.7", "0.3", "1024", "128000"];
            (labels[i % 5], values[i % 5])
        })
        .collect();

    let mut b = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::fixed(size))
            .height(Sizing::fixed(size))
            .padding(Padding::all(8.0))
            .gap(5.0),
    );
    // Header: Grow height 10..20
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::grow_range(10.0, 20.0))
            .padding(Padding::new(5.0, 5.0, 4.0, 4.0))
            .align_y(AlignY::Center),
        |b| {
            b.with(El::row().width(Sizing::GROW).height(Sizing::FIT), |b| {
                b.with(El::new().width(Sizing::FIT).height(Sizing::GROW), |b| {
                    b.text(("STATUS", TextStyle::new(10.0)));
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
                        b.text(("BENCH", TextStyle::new(10.0)));
                    },
                );
            });
        },
    );
    // Divider: Fixed 4
    b.with(
        El::new().width(Sizing::GROW).height(Sizing::fixed(4.0)),
        |_| {},
    );
    // Body: Grow
    b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |b| {
        b.with(
            El::column()
                .width(Sizing::GROW)
                .padding(Padding::all(5.0))
                .gap(2.0),
            |b| {
                for (label, value) in &rows {
                    b.with(El::row().width(Sizing::GROW).height(Sizing::FIT), |b| {
                        b.text((*label, TextStyle::new(10.0)));
                        b.with(
                            El::new().width(Sizing::GROW).height(Sizing::fixed(1.0)),
                            |_| {},
                        );
                        b.text((*value, TextStyle::new(10.0)));
                    });
                }
            },
        );
    });
    let tree = b.build();
    let engine = LayoutEngine::new(measure);
    let result = engine.compute(&tree, size, size, 1.0);

    // Root children by builder order: header (1), divider (8), body (9).
    // Available height = 160 - 16 (padding) - 10 (2 gaps) = 134.
    // Header: GROW(10..20), content = 18 (4+10+4 padding + 10px text) → 18.
    // Divider: Fixed(4) → 4.
    // Body: GROW, 20 rows of text → content height 248.
    // Content overflows the panel (18 + 4 + 248 = 270 > 134). The layout
    // engine keeps the content height rather than compressing the body;
    // render-side systems decide what is visible in the current viewport.
    let header = &result.computed[1];
    let divider = &result.computed[8];
    let body = &result.computed[9];

    assert_eq!(
        header.bounds.height, 18.0,
        "header height (content within [10, 20])"
    );
    assert_eq!(divider.bounds.height, 4.0, "divider height (Fixed)");
    assert_eq!(
        body.bounds.height, 248.0,
        "body height (full content, overflows panel)"
    );
}

/// A one-element Fit text panel at the standard viewport — the one-element panel
/// a standalone label becomes at runtime, the geometry-stable skip's main subject.
fn fit_text_tree(text: &str) -> LayoutTree {
    let mut builder = LayoutBuilder::new(VIEWPORT, VIEWPORT);
    builder.text((text, TextStyle::new(10.0)));
    builder.build()
}

fn fit_measure_as_tree(text: &str, measurement_text: &str) -> LayoutTree {
    let mut builder = LayoutBuilder::new(VIEWPORT, VIEWPORT);
    builder.text(Text::new(text, TextStyle::new(10.0)).measure_as(measurement_text));
    builder.build()
}

#[test]
fn can_reuse_geometry_holds_for_a_same_width_text_swap_only() {
    let measure = monospace_measure();
    let engine = LayoutEngine::new(Arc::clone(&measure));
    let result = engine.compute(&fit_text_tree("AAA"), VIEWPORT, VIEWPORT, 1.0);

    // A same-char-count monospace swap measures bit-identical, so the cached
    // geometry still describes the box — the skip is allowed.
    assert!(
        result.can_reuse_geometry(&fit_text_tree("BBB"), &measure, VIEWPORT, VIEWPORT, 1.0),
        "an equal-width retext should reuse geometry",
    );

    // A wider string measures differently, so the box would move — solve.
    assert!(
        !result.can_reuse_geometry(&fit_text_tree("AAAA"), &measure, VIEWPORT, VIEWPORT, 1.0),
        "a wider retext must not reuse geometry",
    );

    // A viewport change re-positions content even at identical text — solve.
    assert!(
        !result.can_reuse_geometry(
            &fit_text_tree("BBB"),
            &measure,
            VIEWPORT + 1.0,
            VIEWPORT,
            1.0
        ),
        "a viewport change must not reuse geometry",
    );
}

#[test]
fn can_reuse_geometry_holds_for_measure_as_text_swap() {
    let measure = monospace_measure();
    let engine = LayoutEngine::new(Arc::clone(&measure));
    let result = engine.compute(
        &fit_measure_as_tree("OK", "Disconnected"),
        VIEWPORT,
        VIEWPORT,
        1.0,
    );

    assert!(
        result.can_reuse_geometry(
            &fit_measure_as_tree("Ready", "Disconnected"),
            &measure,
            VIEWPORT,
            VIEWPORT,
            1.0
        ),
        "a measure_as retext should reuse geometry when the surrogate is unchanged",
    );
    assert!(
        !result.can_reuse_geometry(
            &fit_measure_as_tree("Ready", "Connected"),
            &measure,
            VIEWPORT,
            VIEWPORT,
            1.0
        ),
        "a different surrogate measurement must relayout",
    );
}

#[test]
fn can_reuse_geometry_rejects_a_newline_even_at_the_same_natural_width() {
    let measure = monospace_measure();
    let engine = LayoutEngine::new(Arc::clone(&measure));
    let result = engine.compute(&fit_text_tree("AAA"), VIEWPORT, VIEWPORT, 1.0);

    // "AAA\nA" has the same widest-line width as "AAA", so the width check alone
    // would pass — but the newline forces a second line, which the unwrapped
    // single-command reuse cannot render. The explicit newline guard rejects it.
    assert!(
        !result.can_reuse_geometry(&fit_text_tree("AAA\nA"), &measure, VIEWPORT, VIEWPORT, 1.0),
        "new text with a newline must not reuse single-line geometry",
    );
}

#[test]
fn can_reuse_geometry_rejects_a_wrapped_leaf() {
    let measure = monospace_measure();
    let engine = LayoutEngine::new(Arc::clone(&measure));
    // A fixed-width element narrower than the text forces word wrapping, so the
    // result caches per-line breaks for the old string.
    let mut builder = LayoutBuilder::new(VIEWPORT, VIEWPORT);
    builder.with(
        El::new().width(Sizing::fixed(40.0)).height(Sizing::FIT),
        |builder| {
            builder.text(
                Text::new("alpha beta gamma delta", TextStyle::new(10.0)).wrap(TextWrap::Words),
            );
        },
    );
    let tree = builder.build();
    let result = engine.compute(&tree, VIEWPORT, VIEWPORT, 1.0);

    // Even re-checking the identical tree must decline: the cached lines belong
    // to the prior string and cannot carry a new one through `regenerate`.
    assert!(
        !result.can_reuse_geometry(&tree, &measure, VIEWPORT, VIEWPORT, 1.0),
        "a wrapped leaf can never reuse its stale line breaks",
    );
}
