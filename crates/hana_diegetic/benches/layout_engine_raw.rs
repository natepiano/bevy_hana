#![allow(
    clippy::expect_used,
    reason = "benchmarks expect valid fixtures and use black_box for output"
)]

//! Raw `LayoutEngine` and diagnostic micro-benchmarks.
//!
//! This target is gated behind `bench_support` because it intentionally
//! touches crate internals that are not part of the normal public API.
//!
//! Slices per row count:
//! - **`build_tree_only`**: public `LayoutBuilder` tree construction.
//! - **`scale_tree_only`**: unit conversion via `LayoutTree::scaled`.
//! - **`raw_compute_prebuilt_tree`**: `LayoutEngine::compute` on an already built and scaled tree.
//! - **`raw_compute_prebuilt_unscaled_tree`**: `LayoutEngine::compute` on an already built unscaled
//!   tree, matching the direct layout-comparison viewport and font scale.
//! - **`build_compute_unscaled_tree`**: build the unscaled tree and immediately compute it,
//!   matching the Diegetic side of `layout_comparison`.
//! - **`regenerate_commands_only`**: visual-only command regeneration from cached geometry.
//! - **`layout_tree_diff_*`**: field-by-field tree change classification cost.
//!
//! Run with `cargo bench --bench layout_engine_raw --features bench_support`.

mod fixtures;

use std::hint::black_box;

use bevy::color::Color;
use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use fixtures::measurement::monospace_measure_text_fn;
use fixtures::panels::DiegeticStatusTreeStyle;
use fixtures::panels::PANEL_SIZE;
use fixtures::panels::build_diegetic_status_tree;
use fixtures::panels::build_diegetic_status_tree_with_style;
use fixtures::panels::layout_to_points;
use fixtures::rows::ROW_COUNTS;
use fixtures::rows::StatusRow;
use fixtures::rows::generate_status_rows;
use hana_diegetic::bench_support::LayoutEngine;
use hana_diegetic::bench_support::LayoutTreeChange;

fn bench_raw_layout(c: &mut Criterion) {
    let scale = layout_to_points(PANEL_SIZE);
    let viewport_size = PANEL_SIZE * scale;

    for row_count in ROW_COUNTS {
        let rows = generate_status_rows(row_count);
        bench_raw_status_panel(c, row_count, &rows, scale, viewport_size);
        bench_layout_tree_diff(c, row_count, &rows);
    }
}

fn bench_raw_status_panel(
    c: &mut Criterion,
    row_count: usize,
    rows: &[StatusRow],
    scale: f32,
    viewport_size: f32,
) {
    let group_name = format!("raw_status_panel_{row_count}_rows");
    let mut group = c.benchmark_group(&group_name);

    group.bench_function("build_tree_only", |b| {
        b.iter(|| {
            let tree = build_diegetic_status_tree(black_box(rows));
            black_box(tree);
        });
    });

    group.bench_function("scale_tree_only", |b| {
        let tree = build_diegetic_status_tree(rows);

        b.iter(|| {
            let scaled_tree = tree.scaled(black_box(scale), black_box(scale));
            black_box(scaled_tree);
        });
    });

    group.bench_function("raw_compute_prebuilt_tree", |b| {
        let tree = build_diegetic_status_tree(rows).scaled(scale, scale);
        let engine = LayoutEngine::new(monospace_measure_text_fn());

        b.iter(|| {
            let result = engine.compute(
                black_box(&tree),
                black_box(viewport_size),
                black_box(viewport_size),
                black_box(1.0),
            );
            black_box(result);
        });
    });

    group.bench_function("raw_compute_prebuilt_unscaled_tree", |b| {
        let tree = build_diegetic_status_tree(rows);
        let engine = LayoutEngine::new(monospace_measure_text_fn());

        b.iter(|| {
            let result = engine.compute(
                black_box(&tree),
                black_box(PANEL_SIZE),
                black_box(PANEL_SIZE),
                black_box(1.0),
            );
            black_box(result);
        });
    });

    group.bench_function("build_compute_unscaled_tree", |b| {
        let engine = LayoutEngine::new(monospace_measure_text_fn());

        b.iter(|| {
            let tree = build_diegetic_status_tree(black_box(rows));
            let result = engine.compute(
                black_box(&tree),
                black_box(PANEL_SIZE),
                black_box(PANEL_SIZE),
                black_box(1.0),
            );
            black_box(result);
        });
    });

    group.bench_function("regenerate_commands_only", |b| {
        let tree = build_diegetic_status_tree(rows).scaled(scale, scale);
        let text_color_tree = build_diegetic_status_tree_with_style(
            rows,
            DiegeticStatusTreeStyle {
                text_color: Color::srgb(0.2, 0.8, 1.0),
                ..Default::default()
            },
        )
        .scaled(scale, scale);
        let engine = LayoutEngine::new(monospace_measure_text_fn());
        let base_result = engine.compute(&tree, viewport_size, viewport_size, 1.0);

        b.iter(|| {
            let mut result = black_box(base_result.clone());
            result.regenerate_commands(black_box(&text_color_tree));
            black_box(result);
        });
    });

    group.finish();
}

fn bench_layout_tree_diff(c: &mut Criterion, row_count: usize, rows: &[StatusRow]) {
    let base_tree = build_diegetic_status_tree(rows);
    let identical_tree = base_tree.clone();
    let text_color_tree = build_diegetic_status_tree_with_style(
        rows,
        DiegeticStatusTreeStyle {
            text_color: Color::srgb(0.2, 0.8, 1.0),
            ..Default::default()
        },
    );
    let background_color_tree = build_diegetic_status_tree_with_style(
        rows,
        DiegeticStatusTreeStyle {
            root_background: Color::srgb_u8(96, 122, 180),
            ..Default::default()
        },
    );
    let layout_change_early_tree = build_diegetic_status_tree_with_style(
        rows,
        DiegeticStatusTreeStyle {
            root_child_gap: 6.0,
            ..Default::default()
        },
    );
    let mut late_rows = rows.to_vec();
    if let Some((_, value)) = late_rows.last_mut() {
        *value = "changed";
    }
    let layout_change_late_tree = build_diegetic_status_tree(&late_rows);

    assert_tree_diff_fixtures(
        &base_tree,
        &identical_tree,
        &text_color_tree,
        &background_color_tree,
        &layout_change_early_tree,
        &layout_change_late_tree,
    );

    let diff_group_name = format!("layout_tree_diff_{row_count}_rows");
    let mut group = c.benchmark_group(&diff_group_name);

    group.bench_function("compare_identical_tree", |b| {
        b.iter(|| {
            let change = black_box(&base_tree).classify_change(black_box(&identical_tree));
            black_box(change);
        });
    });

    group.bench_function("compare_text_color_only_tree", |b| {
        b.iter(|| {
            let change = black_box(&base_tree).classify_change(black_box(&text_color_tree));
            black_box(change);
        });
    });

    group.bench_function("compare_background_color_only_tree", |b| {
        b.iter(|| {
            let change = black_box(&base_tree).classify_change(black_box(&background_color_tree));
            black_box(change);
        });
    });

    group.bench_function("compare_layout_change_early_exit", |b| {
        b.iter(|| {
            let change =
                black_box(&base_tree).classify_change(black_box(&layout_change_early_tree));
            black_box(change);
        });
    });

    group.bench_function("compare_layout_change_late_exit", |b| {
        b.iter(|| {
            let change = black_box(&base_tree).classify_change(black_box(&layout_change_late_tree));
            black_box(change);
        });
    });

    group.finish();
}

fn assert_tree_diff_fixtures(
    base_tree: &hana_diegetic::LayoutTree,
    identical_tree: &hana_diegetic::LayoutTree,
    text_color_tree: &hana_diegetic::LayoutTree,
    background_color_tree: &hana_diegetic::LayoutTree,
    layout_change_early_tree: &hana_diegetic::LayoutTree,
    layout_change_late_tree: &hana_diegetic::LayoutTree,
) {
    assert_eq!(
        base_tree.classify_change(identical_tree),
        LayoutTreeChange::Identical
    );
    assert_eq!(
        base_tree.classify_change(text_color_tree),
        LayoutTreeChange::VisualOnly
    );
    assert_eq!(
        base_tree.classify_change(background_color_tree),
        LayoutTreeChange::VisualOnly
    );
    assert_eq!(
        base_tree.classify_change(layout_change_early_tree),
        LayoutTreeChange::LayoutAffecting
    );
    assert_eq!(
        base_tree.classify_change(layout_change_late_tree),
        LayoutTreeChange::LayoutAffecting
    );
}

criterion_group!(benches, bench_raw_layout);
criterion_main!(benches);
