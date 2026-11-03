//! The service reports complete, frame-matched wall-time breakdowns.
#![allow(clippy::expect_used, reason = "tests stop on an absent measured span")]

use std::time::Duration;
use std::time::Instant;

use bevy_app::App;
use bevy_app::Last;
use bevy_app::MainScheduleOrder;
use bevy_app::Update;
use bevy_diagnostic::FrameCountPlugin;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::schedule::IntoSystemSet;
use bevy_ecs::schedule::NodeId;
use bevy_ecs::schedule::ScheduleLabel;
use bevy_ecs::schedule::SystemSet;
use bevy_ecs::world::World;
use bevy_render::Render;
use bevy_render::RenderApp;
use bevy_render::RenderSystems;
use bevy_render::extract_plugin::ExtractPlugin;
use bevy_render::renderer::RenderGraph;
use bevy_render::renderer::RenderGraphSystems;
use bevy_render::view::prepare_view_attachments;
use bevy_render::view::prepare_view_targets;
use bevy_render::view::prepare_windows;
use bevy_state::app::StatesPlugin;
use bevy_state::state::StateTransition;
use hana_metrics::CurrentSpanReading;
use hana_metrics::FrameTiming;
use hana_metrics::FrameTimingPlugin;
use hana_metrics::SpanSample;
use hana_metrics::SpanTiming;

/// Main-thread work scheduled after `Last`, as BRP's `RemoteLast` is.
#[derive(ScheduleLabel, Clone, Debug, Hash, PartialEq, Eq)]
struct AfterLastWork;

fn spin_for(duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {
        std::hint::spin_loop();
    }
}

fn spin_briefly() { spin_for(Duration::from_micros(100)); }

fn spin_for_two_milliseconds() { spin_for(Duration::from_millis(2)); }

fn spin_for_four_milliseconds() { spin_for(Duration::from_millis(4)); }

fn add_timing(app: &mut App) { app.add_plugins((FrameCountPlugin, FrameTimingPlugin)); }

fn timed_app() -> App {
    let mut app = App::new();
    add_timing(&mut app);
    app
}

/// Finishes plugin setup as Bevy's runner does, then runs until a record is
/// complete.
fn settled_record(app: &mut App) -> &FrameTiming {
    app.finish();
    app.cleanup();
    // A complete interval ends at the following frame's first mark. Leave one
    // more update for the main/render publications to meet on that frame key.
    for _ in 0..4 {
        app.update();
    }
    app.world().resource::<FrameTiming>()
}

fn run_render_graph(world: &mut World) { world.run_schedule(RenderGraph); }

/// A render app with work in every stage the service times, before timing is
/// added.
fn render_fixture() -> App {
    let mut app = App::new();
    app.add_plugins(ExtractPlugin::<RenderApp>::new(
        |_, _| {},
        Render::base_schedule,
        Render.intern(),
        RenderSystems::ExtractCommands.intern(),
        RenderSystems::PostCleanup.intern(),
    ));
    let render = app.sub_app_mut(RenderApp);
    render.add_schedule(RenderGraph::base_schedule());
    render.update_schedule = Some(Render.intern());
    render.add_systems(
        Render,
        (
            spin_briefly.in_set(RenderSystems::ExtractCommands),
            spin_briefly.in_set(RenderSystems::PrepareAssets),
            spin_briefly.in_set(RenderSystems::PrepareViews),
            spin_briefly.in_set(RenderSystems::Prepare),
            run_render_graph.in_set(RenderSystems::Render),
            spin_briefly.in_set(RenderSystems::Cleanup),
        ),
    );
    render.add_systems(
        RenderGraph,
        (
            spin_briefly.in_set(RenderGraphSystems::Render),
            spin_briefly.in_set(RenderGraphSystems::Submit),
            spin_briefly.in_set(RenderGraphSystems::Finish),
        ),
    );
    app
}

fn timed_render_app() -> App {
    let mut app = render_fixture();
    add_timing(&mut app);
    app
}

/// Main schedule rows an app without states runs.
const fn stateless_main_spans(timing: &FrameTiming) -> [(&'static str, &SpanTiming); 7] {
    [
        ("First", &timing.first),
        ("PreUpdate", &timing.pre_update),
        ("RunFixedMainLoop", &timing.fixed_update),
        ("Update", &timing.update),
        ("SpawnScene", &timing.spawn_scene),
        ("PostUpdate", &timing.post_update),
        ("Last", &timing.last),
    ]
}

const fn render_spans(timing: &FrameTiming) -> [(&'static str, &SpanTiming); 7] {
    [
        ("extract commands", &timing.extract_commands),
        ("prepare assets", &timing.prepare_assets),
        ("waiting for next image", &timing.waiting_for_next_image),
        ("prepare and queue", &timing.prepare_and_queue),
        ("render graph", &timing.render_graph),
        ("submit", &timing.submit),
        ("cleanup", &timing.cleanup),
    ]
}

const fn measured(span: &SpanTiming) -> (u32, f32) {
    match span.sample {
        SpanSample::Measured { frame, last_ms, .. } => Some((frame, last_ms)),
        SpanSample::NeverMeasured => None,
    }
    .expect("expected a measured span")
}

const fn sample_frame(span: &SpanTiming) -> u32 { measured(span).0 }

const fn last_ms(span: &SpanTiming) -> f32 { measured(span).1 }

fn assert_partition(timing: &FrameTiming, accounted_ms: f32) {
    assert!(
        (accounted_ms - last_ms(&timing.frame_time)).abs() <= 0.01,
        "one frame accounts for {accounted_ms} ms of {} ms",
        last_ms(&timing.frame_time)
    );
}

#[test]
fn update_span_covers_a_two_millisecond_system() {
    let mut app = timed_app();
    app.add_systems(Update, spin_for_two_milliseconds);

    let timing = settled_record(&mut app);
    assert!(
        last_ms(&timing.update) >= 2.0,
        "Update measured {} ms",
        last_ms(&timing.update)
    );
    assert_eq!(sample_frame(&timing.update), timing.frame);
}

#[test]
fn main_rows_and_other_partition_a_frame_without_states() {
    let mut app = timed_app();
    let timing = settled_record(&mut app);
    assert!(last_ms(&timing.frame_time) > 0.0);
    assert_eq!(sample_frame(&timing.frame_time), timing.frame);
    assert_eq!(timing.state_transition.sample, SpanSample::NeverMeasured);
    for (schedule, span) in stateless_main_spans(timing) {
        assert_eq!(
            sample_frame(span),
            timing.frame,
            "{schedule} belongs to another frame"
        );
        assert!(
            last_ms(span).is_finite(),
            "{schedule} has a nonfinite duration"
        );
        assert!(last_ms(span) >= 0.0, "{schedule} has a negative duration");
    }
    assert_eq!(sample_frame(&timing.other), timing.frame);
    assert!(last_ms(&timing.other) >= 0.0, "unmeasured time is negative");

    let accounted_ms = stateless_main_spans(timing)
        .iter()
        .map(|(_, span)| last_ms(span))
        .sum::<f32>()
        + last_ms(&timing.other);
    assert_partition(timing, accounted_ms);
}

#[test]
fn render_wait_and_extract_join_the_partition_of_a_rendered_frame() {
    let mut app = timed_render_app();
    let timing = settled_record(&mut app);
    let between_frames = [
        ("waiting for render", &timing.waiting_for_render),
        ("extract", &timing.extract),
        ("other", &timing.other),
    ];
    for (row, span) in stateless_main_spans(timing)
        .into_iter()
        .chain(between_frames)
    {
        assert_eq!(
            sample_frame(span),
            timing.frame,
            "{row} belongs to another frame"
        );
        assert!(last_ms(span) >= 0.0, "{row} has a negative duration");
    }

    let accounted_ms = stateless_main_spans(timing)
        .into_iter()
        .chain(between_frames)
        .map(|(_, span)| last_ms(span))
        .sum::<f32>();
    assert_partition(timing, accounted_ms);
}

#[test]
fn render_app_populates_every_render_stage_for_its_frame() {
    let mut app = timed_render_app();
    let timing = settled_record(&mut app);
    for (stage, span) in render_spans(timing) {
        assert_eq!(
            sample_frame(span),
            timing.frame,
            "{stage} belongs to another frame"
        );
        assert!(
            last_ms(span).is_finite(),
            "{stage} has a nonfinite duration"
        );
        assert!(last_ms(span) >= 0.0, "{stage} has a negative duration");
    }
}

#[test]
fn work_scheduled_after_last_counts_in_the_last_row() {
    let mut app = render_fixture();
    app.world_mut()
        .resource_mut::<MainScheduleOrder>()
        .insert_after(Last, AfterLastWork);
    app.add_systems(AfterLastWork, spin_for_four_milliseconds);
    add_timing(&mut app);

    let timing = settled_record(&mut app);
    assert!(
        last_ms(&timing.last) >= 4.0,
        "Last measured {} ms around 4 ms of work after it",
        last_ms(&timing.last)
    );
    assert_eq!(sample_frame(&timing.last), timing.frame);
}

#[test]
fn states_added_after_the_plugin_run_state_transitions_once() {
    let mut app = timed_app();
    app.add_plugins(StatesPlugin);
    let timing = settled_record(&mut app);
    let (record_frame, state_transition_frame) =
        (timing.frame, sample_frame(&timing.state_transition));

    let state_transitions = app
        .world()
        .resource::<MainScheduleOrder>()
        .labels
        .iter()
        .filter(|label| **label == StateTransition.intern())
        .count();
    assert_eq!(
        state_transitions, 1,
        "StateTransition runs {state_transitions} times a frame"
    );
    assert_eq!(
        state_transition_frame, record_frame,
        "StateTransition was not timed"
    );
}

#[test]
fn rows_without_a_reading_remain_never_measured() {
    let mut app = timed_app();
    let timing = settled_record(&mut app);
    assert!(timing.frame > 0, "no record completed");
    assert_eq!(sample_frame(&timing.first), timing.frame);
    assert_eq!(sample_frame(&timing.last), timing.frame);
    let unread = [
        ("waiting for render", &timing.waiting_for_render),
        ("extract", &timing.extract),
    ];
    for (row, span) in unread.into_iter().chain(render_spans(timing)) {
        assert_eq!(span.sample, SpanSample::NeverMeasured, "{row} was measured");
    }
}

#[test]
fn a_span_keeps_its_last_measured_sample_when_no_reading_arrives() {
    let mut app = timed_app();
    let first = settled_record(&mut app);
    let first_frame = first.frame;
    let sample = SpanSample::Measured {
        frame:          first_frame,
        last_ms:        0.4,
        smoothed_ms:    0.3,
        measured_at_ms: first.published_at_ms,
    };

    app.world_mut()
        .resource_mut::<FrameTiming>()
        .state_transition
        .sample = sample;
    for _ in 0..3 {
        app.update();
    }
    let later = app.world().resource::<FrameTiming>();
    assert!(later.frame > first_frame);
    assert_eq!(later.state_transition.sample, sample);
}

#[test]
fn current_reading_requires_a_measurement_within_five_seconds() {
    let mut span = SpanTiming::default();
    assert_eq!(span.current_reading(6000), CurrentSpanReading::Unavailable);

    span.sample = SpanSample::Measured {
        frame:          1,
        last_ms:        0.4,
        smoothed_ms:    0.3,
        measured_at_ms: 1000,
    };
    span.peak_5s_ms = 1.25;
    assert_eq!(
        span.current_reading(6000),
        CurrentSpanReading::Measured {
            smoothed_ms: 0.3,
            peak_5s_ms:  1.25,
        }
    );
    assert_eq!(span.current_reading(6001), CurrentSpanReading::Unavailable);
}

#[test]
fn view_preparation_after_the_image_acquire_counts_as_prepare_and_queue() {
    let mut app = render_fixture();
    // The real acquire and view preparation need a GPU. Keep their production
    // ordering with disabled systems and put the measured work after them.
    app.sub_app_mut(RenderApp).add_systems(
        Render,
        (
            prepare_windows
                .in_set(RenderSystems::PrepareViews)
                .run_if(|| false),
            prepare_view_attachments
                .in_set(RenderSystems::PrepareViews)
                .after(prepare_windows)
                .before(prepare_view_targets)
                .run_if(|| false),
            prepare_view_targets
                .in_set(RenderSystems::PrepareViews)
                .after(prepare_windows)
                .run_if(|| false),
            spin_for_four_milliseconds
                .in_set(RenderSystems::PrepareViews)
                .after(prepare_view_targets),
        ),
    );
    add_timing(&mut app);

    let timing = settled_record(&mut app);
    assert_eq!(sample_frame(&timing.prepare_and_queue), timing.frame);
    assert_eq!(sample_frame(&timing.waiting_for_next_image), timing.frame);
    // The view work runs after the image-wait end marker, so load can only
    // lengthen what prepare and queue measures around it.
    assert!(
        last_ms(&timing.prepare_and_queue) >= 2.0,
        "prepare and queue measured {} ms around 4 ms of view work",
        last_ms(&timing.prepare_and_queue)
    );

    let schedule = app
        .sub_app(RenderApp)
        .get_schedule(Render)
        .expect("the render schedule was built");
    let graph = schedule.graph();
    let end = schedule
        .systems()
        .expect("the render schedule has executable systems")
        .find_map(|(key, system)| {
            system
                .name()
                .ends_with("::mark_after_image")
                .then_some(NodeId::System(key))
        })
        .expect("the production image-wait end marker is installed");
    // Merely ordering the end marker and view work after prepare_windows
    // leaves them unordered with respect to each other. These edges prevent
    // view work being subtracted as image wait, regardless of executor/load.
    for views in [
        prepare_view_attachments.into_system_set().intern(),
        prepare_view_targets.into_system_set().intern(),
    ] {
        let views = NodeId::Set(
            graph
                .system_sets
                .get_key(views)
                .expect("the production view system set is present"),
        );
        assert!(
            graph.dependency().graph().contains_edge(end, views),
            "the image-wait end marker must precede {views:?}, so view work counts as prepare and queue"
        );
    }
}
