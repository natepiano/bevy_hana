//! Benchmark comparing Clay (FFI) and `hana_diegetic` layout performance on
//! identical panel layouts.
//!
//! - **`clay_immediate_build_layout_collect`**: immediate-mode build + layout + command collection
//!   using a reused `Clay` context.
//! - **`diegetic_build_compute`**: build the same public `LayoutTree` fixture, run
//!   `LayoutEngine::compute`, and read the `LayoutResult`.
//!
//! # Methodology notes
//!
//! This benchmark compares direct layout passes without Bevy ECS scheduling,
//! `DiegeticPanel` change detection, panel unit scaling, or render-system work.
//! It exists alongside `panel_perf`, which measures the public Bevy panel path,
//! and `layout_engine_raw`, which breaks diegetic costs into smaller diagnostic
//! slices.
//!
//! Run with `cargo bench -p hana_diegetic --bench layout_comparison --features bench_support`.

mod fixtures;

use std::hint::black_box;

use clay_layout::Clay;
use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use fixtures::measurement::clay_monospace_measure;
use fixtures::measurement::monospace_measure_text_fn;
use fixtures::panels::PANEL_SIZE;
use fixtures::panels::build_clay_status_panel;
use fixtures::panels::build_diegetic_status_tree;
use fixtures::rows::ROW_COUNTS;
use fixtures::rows::StatusRow;
use fixtures::rows::generate_status_rows;
use hana_diegetic::bench_support::LayoutEngine;

fn run_clay_layout(clay: &mut Clay, rows: &[StatusRow]) {
    let mut layout = clay.begin::<(), ()>();
    build_clay_status_panel(&mut layout, rows);
    let commands: Vec<_> = layout.end().collect();
    black_box(&commands);
}

fn run_diegetic_layout(engine: &LayoutEngine, rows: &[StatusRow]) {
    let tree = build_diegetic_status_tree(rows);
    let result = engine.compute(&tree, PANEL_SIZE, PANEL_SIZE, 1.0);
    black_box(result);
}

fn bench_status_panel(c: &mut Criterion) {
    for row_count in ROW_COUNTS {
        let rows = generate_status_rows(row_count);
        let group_name = format!("status_panel_{row_count}_rows");
        let mut group = c.benchmark_group(&group_name);

        group.bench_function("clay_immediate_build_layout_collect", |b| {
            let mut clay = Clay::new((PANEL_SIZE, PANEL_SIZE).into());
            clay.set_measure_text_function_user_data((), clay_monospace_measure);

            b.iter(|| run_clay_layout(&mut clay, black_box(&rows)));
        });

        group.bench_function("diegetic_build_compute", |b| {
            let engine = LayoutEngine::new(monospace_measure_text_fn());

            b.iter(|| run_diegetic_layout(&engine, black_box(&rows)));
        });

        group.finish();
    }
}

criterion_group!(benches, bench_status_panel);
criterion_main!(benches);
