use std::sync::atomic::Ordering;
use std::time::Instant;

use bevy_app::App;
use bevy_ecs::prelude::IntoScheduleConfigs;
use bevy_ecs::prelude::Res;
use bevy_ecs::prelude::ResMut;
use bevy_ecs::prelude::Resource;
use bevy_render::Render;
use bevy_render::RenderApp;
use bevy_render::RenderSystems;
use bevy_render::renderer::RenderGraph;
use bevy_render::renderer::RenderGraphSystems;
use bevy_render::view::create_surfaces;
use bevy_render::view::prepare_view_attachments;
use bevy_render::view::prepare_view_targets;
use bevy_render::view::prepare_windows;

use crate::Reading;
use crate::SharedTiming;
use crate::duration_ms;
use crate::main_thread::MainFrameMark;

#[derive(Resource)]
struct RenderMarks {
    frame:         u32,
    start:         Instant,
    before_assets: Instant,
    after_assets:  Instant,
    before_image:  Instant,
    after_image:   Instant,
    before_graph:  Instant,
    after_graph:   Instant,
}

#[derive(Resource)]
struct RenderGraphMarks {
    start:        Instant,
    after_render: Instant,
    after_submit: Instant,
    complete:     bool,
}

pub(super) fn install(app: &mut App, shared: SharedTiming) {
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    let now = Instant::now();
    if let Some(mut original_extract) = render_app.take_extract() {
        let extraction = shared.clone();
        render_app.set_extract(move |main_world, render_world| {
            let mark = main_world.resource::<MainFrameMark>();
            let frame = mark.frame;
            let now = Instant::now();
            let wait = duration_ms(mark.last_end, now);
            let slot = frame as usize % 3;
            extraction.0.wait_ms[slot].store(wait.to_bits(), Ordering::Relaxed);
            extraction.0.wait_frame[slot].store(frame, Ordering::Release);
            extraction.0.extract_frame.store(frame, Ordering::Relaxed);
            original_extract(main_world, render_world);
            let elapsed = duration_ms(now, Instant::now());
            extraction.0.extraction_ms[slot].store(elapsed.to_bits(), Ordering::Relaxed);
            extraction.0.extraction_frame[slot].store(frame, Ordering::Release);
        });
    }
    render_app.insert_resource(shared);
    render_app.insert_resource(RenderMarks {
        frame:         u32::MAX,
        start:         now,
        before_assets: now,
        after_assets:  now,
        before_image:  now,
        after_image:   now,
        before_graph:  now,
        after_graph:   now,
    });
    render_app.insert_resource(RenderGraphMarks {
        start:        now,
        after_render: now,
        after_submit: now,
        complete:     false,
    });
    render_app.add_systems(
        Render,
        // Since Bevy 0.20 the `RenderSystems` sets are ordered weakly. Each mark is
        // ordered only against the sets it names and against conflicting,
        // deferred or exclusive systems, so a reading is the wall-clock span
        // between two marks, not the exclusive time of one set.
        (
            mark_render_start.before(RenderSystems::ExtractCommands),
            mark_before_assets
                .after(RenderSystems::ExtractCommands)
                .before(RenderSystems::PrepareAssets),
            mark_after_assets
                .after(RenderSystems::PrepareAssets)
                .before(RenderSystems::PrepareMeshes),
            // Only the acquire is the wait; the rest of `PrepareViews` is
            // prepare and queue work.
            mark_before_image
                .in_set(RenderSystems::PrepareViews)
                .after(create_surfaces)
                .before(prepare_windows),
            mark_after_image
                .in_set(RenderSystems::PrepareViews)
                .after(prepare_windows)
                // Sharing an after-acquire edge leaves view work unordered
                // with this mark and can subtract it as image wait.
                .before(prepare_view_attachments)
                .before(prepare_view_targets),
            mark_before_graph
                .after(RenderSystems::Prepare)
                .before(RenderSystems::Render),
            mark_after_graph
                .after(RenderSystems::Render)
                .before(RenderSystems::Cleanup),
            publish_render.after(RenderSystems::PostCleanup),
        ),
    );
    render_app.add_systems(
        RenderGraph,
        (
            mark_graph_start.in_set(RenderGraphSystems::Begin),
            mark_graph_after_render
                .after(RenderGraphSystems::Render)
                .before(RenderGraphSystems::Submit),
            mark_graph_after_submit
                .after(RenderGraphSystems::Submit)
                .before(RenderGraphSystems::Finish),
        ),
    );
}

fn mark_render_start(mut marks: ResMut<RenderMarks>, shared: Res<SharedTiming>) {
    marks.start = Instant::now();
    marks.frame = shared.0.extract_frame.load(Ordering::Relaxed);
}

fn mark_before_assets(mut marks: ResMut<RenderMarks>) { marks.before_assets = Instant::now(); }
fn mark_after_assets(mut marks: ResMut<RenderMarks>) { marks.after_assets = Instant::now(); }
fn mark_before_image(mut marks: ResMut<RenderMarks>) { marks.before_image = Instant::now(); }
fn mark_after_image(mut marks: ResMut<RenderMarks>) { marks.after_image = Instant::now(); }
fn mark_before_graph(mut marks: ResMut<RenderMarks>) { marks.before_graph = Instant::now(); }
fn mark_after_graph(mut marks: ResMut<RenderMarks>) { marks.after_graph = Instant::now(); }

fn mark_graph_start(mut marks: ResMut<RenderGraphMarks>) {
    marks.start = Instant::now();
    marks.complete = false;
}

fn mark_graph_after_render(mut marks: ResMut<RenderGraphMarks>) {
    marks.after_render = Instant::now();
}

fn mark_graph_after_submit(mut marks: ResMut<RenderGraphMarks>) {
    marks.after_submit = Instant::now();
    marks.complete = true;
}

fn publish_render(
    marks: Res<RenderMarks>,
    graph: Res<RenderGraphMarks>,
    shared: Res<SharedTiming>,
) {
    let end = Instant::now();
    let extract_commands = duration_ms(marks.start, marks.before_assets);
    let prepare_assets = duration_ms(marks.before_assets, marks.after_assets);
    let image_wait = duration_ms(marks.before_image, marks.after_image);
    // Without a render graph run, nothing marks submission.
    let (prepare_and_queue, render_graph, submit, cleanup) = if graph.complete {
        (
            (duration_ms(marks.after_assets, graph.start) - image_wait).max(0.0),
            duration_ms(graph.start, graph.after_render),
            Reading::Measured(duration_ms(graph.after_render, graph.after_submit)),
            duration_ms(graph.after_submit, end),
        )
    } else {
        (
            (duration_ms(marks.after_assets, marks.before_graph) - image_wait).max(0.0),
            duration_ms(marks.before_graph, marks.after_graph),
            Reading::Unmeasured,
            duration_ms(marks.after_graph, end),
        )
    };
    shared.0.publish_render(
        marks.frame,
        [
            Reading::Measured(extract_commands),
            Reading::Measured(prepare_assets),
            Reading::Measured(image_wait),
            Reading::Measured(prepare_and_queue),
            Reading::Measured(render_graph),
            submit,
            Reading::Measured(cleanup),
        ],
    );
}
