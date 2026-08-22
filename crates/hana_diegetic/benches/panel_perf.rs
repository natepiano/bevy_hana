#![allow(
    clippy::expect_used,
    reason = "benchmarks expect on just-spawned entities where None is a test bug"
)]

//! Benchmark for public `DiegeticPanel` layout performance at various sizes.
//!
//! Measures the public `DiegeticPanel` update path: build or reuse a
//! `LayoutTree`, mutate a panel when the scenario calls for it, and run
//! `app.update()` so `compute_panel_layouts` has a chance to execute. This
//! includes retained-mode API-boundary work and Bevy scheduling; it is not a
//! raw `LayoutEngine` benchmark.
//!
//! Scenarios per row count:
//! - **`cold`**: first layout for a fresh panel.
//! - **`no_change_update`**: unchanged panel frame; layout should be skipped.
//! - **`resize_only`**: panel dimensions change while the tree is reused.
//! - **`warm`**: same logical tree rebuilt and assigned every iteration.
//! - **`color_change_full_rebuild`**: same layout structure rebuilt with visual-only text color
//!   changes, forced through the `bench_support` full-layout component setter.
//! - **`visual_only_rebuild`**: same visual-only change through the optimized command API.
//!
//! Run with `cargo bench --bench panel_perf`.

mod fixtures;

use std::hint::black_box;

use bevy::prelude::Color;
use bevy::prelude::error;
use criterion::BenchmarkGroup;
use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::measurement::WallTime;
use fixtures::app::create_bench_app;
use fixtures::panels::PANEL_SIZE;
use fixtures::panels::RESIZED_PANEL_SIZE;
use fixtures::panels::bench_panel;
use fixtures::panels::build_diegetic_status_tree;
use fixtures::panels::build_diegetic_status_tree_with_text_color;
use fixtures::rows::ROW_COUNTS;
use fixtures::rows::StatusRow;
use fixtures::rows::generate_status_rows;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;

type PanelBenchGroup<'a> = BenchmarkGroup<'a, WallTime>;

fn bench_panel_layout(c: &mut Criterion) {
    for row_count in ROW_COUNTS {
        let rows = generate_status_rows(row_count);
        let group_name = format!("panel_{row_count}_rows");
        let mut group = c.benchmark_group(&group_name);

        bench_cold(&mut group, &rows);
        bench_no_change_update(&mut group, &rows);
        bench_resize_only(&mut group, &rows);
        bench_warm(&mut group, &rows);
        bench_color_change_full_rebuild(&mut group, &rows);
        bench_visual_only_rebuild(&mut group, &rows);

        group.finish();
    }
}

fn bench_cold(group: &mut PanelBenchGroup<'_>, rows: &[StatusRow]) {
    group.bench_function("cold", |b| {
        b.iter_with_setup(
            || {
                let mut app = create_bench_app();
                let tree = build_diegetic_status_tree(rows);
                let entity = app.world_mut().spawn(bench_panel(tree, PANEL_SIZE)).id();
                (app, entity)
            },
            |(mut app, entity)| {
                app.update();
                black_box(app.world().get::<ComputedDiegeticPanel>(entity));
            },
        );
    });
}

fn bench_no_change_update(group: &mut PanelBenchGroup<'_>, rows: &[StatusRow]) {
    group.bench_function("no_change_update", |b| {
        let mut app = create_bench_app();
        let tree = build_diegetic_status_tree(rows);
        let entity = app.world_mut().spawn(bench_panel(tree, PANEL_SIZE)).id();
        app.update();

        b.iter(|| {
            app.update();
            black_box(app.world().get::<ComputedDiegeticPanel>(entity));
        });
    });
}

fn bench_resize_only(group: &mut PanelBenchGroup<'_>, rows: &[StatusRow]) {
    group.bench_function("resize_only", |b| {
        let mut app = create_bench_app();
        let tree = build_diegetic_status_tree(rows);
        let entity = app.world_mut().spawn(bench_panel(tree, PANEL_SIZE)).id();
        app.update();

        let mut expanded = false;
        b.iter(|| {
            expanded = !expanded;
            let size = if expanded {
                RESIZED_PANEL_SIZE
            } else {
                PANEL_SIZE
            };
            let mut panel = app
                .world_mut()
                .get_mut::<DiegeticPanel>(entity)
                .expect("entity must exist");
            panel.set_width(size);
            panel.set_height(size);
            app.update();
            black_box(app.world().get::<ComputedDiegeticPanel>(entity));
        });
    });
}

fn bench_warm(group: &mut PanelBenchGroup<'_>, rows: &[StatusRow]) {
    group.bench_function("warm", |b| {
        let mut app = create_bench_app();
        let tree = build_diegetic_status_tree(rows);
        let entity = app.world_mut().spawn(bench_panel(tree, PANEL_SIZE)).id();
        app.update();

        b.iter(|| {
            let tree = build_diegetic_status_tree(rows);
            app.world_mut()
                .get_mut::<DiegeticPanel>(entity)
                .expect("entity must exist")
                .set_tree_full_rebuild(tree);
            app.update();
            black_box(app.world().get::<ComputedDiegeticPanel>(entity));
        });
    });
}

fn bench_color_change_full_rebuild(group: &mut PanelBenchGroup<'_>, rows: &[StatusRow]) {
    group.bench_function("color_change_full_rebuild", |b| {
        let mut app = create_bench_app();
        let tree = build_diegetic_status_tree(rows);
        let entity = app.world_mut().spawn(bench_panel(tree, PANEL_SIZE)).id();
        app.update();

        let mut toggle = false;
        b.iter(|| {
            toggle = !toggle;
            let color = if toggle {
                Color::srgb(1.0, 0.0, 0.0)
            } else {
                Color::srgb(0.0, 0.0, 1.0)
            };
            let tree = build_diegetic_status_tree_with_text_color(rows, color);
            app.world_mut()
                .get_mut::<DiegeticPanel>(entity)
                .expect("entity must exist")
                .set_tree_full_rebuild(tree);
            app.update();
            black_box(app.world().get::<ComputedDiegeticPanel>(entity));
        });
    });
}

fn bench_visual_only_rebuild(group: &mut PanelBenchGroup<'_>, rows: &[StatusRow]) {
    group.bench_function("visual_only_rebuild", |b| {
        let mut app = create_bench_app();
        let tree = build_diegetic_status_tree(rows);
        let entity = app.world_mut().spawn(bench_panel(tree, PANEL_SIZE)).id();
        app.update();

        let mut toggle = false;
        b.iter(|| {
            toggle = !toggle;
            let color = if toggle {
                Color::srgb(1.0, 0.0, 0.0)
            } else {
                Color::srgb(0.0, 0.0, 1.0)
            };
            let tree = build_diegetic_status_tree_with_text_color(rows, color);
            if let Err(error) = app.world_mut().commands().set_tree(entity, tree) {
                error!("failed to replace benchmark panel tree: {error}");
            }
            app.update();
            black_box(app.world().get::<ComputedDiegeticPanel>(entity));
        });
    });
}

criterion_group!(benches, bench_panel_layout);
criterion_main!(benches);
