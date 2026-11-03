use std::array;
use std::time::Instant;

use bevy_app::App;
use bevy_app::First;
use bevy_app::MainScheduleOrder;
use bevy_app::PostUpdate;
use bevy_app::PreUpdate;
use bevy_app::RunFixedMainLoop;
use bevy_app::SpawnScene;
use bevy_app::Update;
use bevy_diagnostic::FrameCount;
use bevy_diagnostic::FrameCountPlugin;
use bevy_ecs::prelude::Res;
use bevy_ecs::prelude::ResMut;
use bevy_ecs::prelude::Resource;
use bevy_ecs::schedule::InternedScheduleLabel;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::schedule::Schedule;
use bevy_ecs::schedule::ScheduleLabel;
use bevy_ecs::schedule::SingleThreadedExecutor;
use bevy_ecs::system::ScheduleSystem;
use bevy_state::state::StateTransition;

use crate::FrameTiming;
use crate::PeakHistory;
use crate::ROWS;
use crate::Reading;
use crate::SharedTiming;
use crate::duration_ms;

/// Main schedule rows, `first` through `last` in `FrameTiming` order.
const MAIN_ROWS: usize = 8;
/// The "Last" row closes the frame after the final schedule in the order.
const LAST_ROW: usize = MAIN_ROWS - 1;

/// Opens a frame; runs before every other main schedule.
#[derive(ScheduleLabel, Clone, Debug, Hash, PartialEq, Eq)]
struct FrameStart;

/// Closes the main row at this index.
#[derive(ScheduleLabel, Clone, Debug, Hash, PartialEq, Eq)]
struct RowEnd(usize);

/// The schedule each main row before "Last" closes after, in row order.
fn row_anchors() -> [InternedScheduleLabel; LAST_ROW] {
    [
        First.intern(),
        PreUpdate.intern(),
        StateTransition.intern(),
        RunFixedMainLoop.intern(),
        Update.intern(),
        SpawnScene.intern(),
        PostUpdate.intern(),
    ]
}

#[derive(Resource)]
pub(super) struct MainFrameMark {
    pub(super) frame:    u32,
    pub(super) last_end: Instant,
}

/// Whether a main row's closing mark is in `MainScheduleOrder`.
#[derive(Clone, Copy)]
enum EndMark {
    Placed,
    Absent,
}

#[derive(Resource)]
struct MainMarks {
    frame:     u32,
    start:     Instant,
    ends:      [Instant; MAIN_ROWS],
    end_marks: [EndMark; MAIN_ROWS],
    active:    bool,
    ready:     PendingFrame,
}

#[derive(Clone, Copy)]
enum PendingFrame {
    Warmup,
    Complete {
        frame:       u32,
        interval_ms: f32,
        schedules:   [Reading; MAIN_ROWS],
    },
}

pub(super) fn build(app: &mut App) {
    if !app.is_plugin_added::<FrameCountPlugin>() {
        app.add_plugins(FrameCountPlugin);
    }
    let now = Instant::now();
    app.insert_resource(MainFrameMark {
        frame:    0,
        last_end: now,
    });
    app.insert_resource(MainMarks {
        frame:     0,
        start:     now,
        ends:      [now; MAIN_ROWS],
        end_marks: [EndMark::Absent; MAIN_ROWS],
        active:    false,
        ready:     PendingFrame::Warmup,
    });
}

/// Places the frame's opening mark first and each row's closing mark after its
/// schedule. Never adds a timed schedule: a row whose schedule is absent,
/// usually `StateTransition` without a states plugin, is not measured.
pub(super) fn install_marks(app: &mut App) {
    let world = app.world_mut();
    let Some(mut order) = world.get_resource_mut::<MainScheduleOrder>() else {
        return;
    };
    let mut end_marks = [EndMark::Absent; MAIN_ROWS];
    order.labels.insert(0, FrameStart.intern());
    for (row, anchor) in row_anchors().into_iter().enumerate() {
        if let Some(index) = order.labels.iter().position(|label| *label == anchor) {
            order.labels.insert(index + 1, RowEnd(row).intern());
            end_marks[row] = EndMark::Placed;
        }
    }
    order.labels.push(RowEnd(LAST_ROW).intern());
    end_marks[LAST_ROW] = EndMark::Placed;
    world.resource_mut::<MainMarks>().end_marks = end_marks;

    add_mark_schedule(app, FrameStart, start_frame);
    for (row, end_mark) in end_marks.into_iter().enumerate().take(LAST_ROW) {
        if matches!(end_mark, EndMark::Placed) {
            add_mark_schedule(app, RowEnd(row), move |mut marks: ResMut<MainMarks>| {
                marks.ends[row] = Instant::now();
            });
        }
    }
    add_mark_schedule(app, RowEnd(LAST_ROW), mark_last);
}

fn add_mark_schedule<M>(
    app: &mut App,
    label: impl ScheduleLabel,
    mark: impl IntoScheduleConfigs<ScheduleSystem, M>,
) {
    // One system per schedule, nine schedules a frame: the single-threaded
    // executor skips the task-pool scope a multi-threaded run opens, as Bevy's
    // own `Main` does.
    let mut schedule = Schedule::new(label);
    schedule.set_executor(SingleThreadedExecutor::new());
    schedule.add_systems(mark);
    app.add_schedule(schedule);
}

fn start_frame(
    frame_count: Res<FrameCount>,
    mut marks: ResMut<MainMarks>,
    mut frame_mark: ResMut<MainFrameMark>,
    shared: Res<SharedTiming>,
    mut timing: ResMut<FrameTiming>,
    mut peaks: ResMut<PeakHistory>,
) {
    let now = Instant::now();
    if let PendingFrame::Complete {
        frame,
        interval_ms,
        schedules,
    } = marks.ready
    {
        let schedule_ms: f32 = schedules.iter().map(|reading| reading.accounted_ms()).sum();
        let waiting = shared
            .0
            .waiting_for_render(frame)
            .clamped(interval_ms - schedule_ms);
        let extraction = shared
            .0
            .extraction(frame)
            .clamped(interval_ms - schedule_ms - waiting.accounted_ms());
        let other_ms =
            (interval_ms - schedule_ms - waiting.accounted_ms() - extraction.accounted_ms())
                .max(0.0);
        let [
            first,
            pre_update,
            state_transition,
            fixed_update,
            update,
            spawn_scene,
            post_update,
            last,
        ] = schedules;
        let [
            extract_commands,
            prepare_assets,
            waiting_for_next_image,
            prepare_and_queue,
            render_graph,
            submit,
            cleanup,
        ] = shared.0.render_for(frame);
        let readings: [Reading; ROWS] = [
            Reading::Measured(interval_ms),
            first,
            pre_update,
            state_transition,
            fixed_update,
            update,
            spawn_scene,
            post_update,
            last,
            waiting,
            extraction,
            Reading::Measured(other_ms),
            extract_commands,
            prepare_assets,
            waiting_for_next_image,
            prepare_and_queue,
            render_graph,
            submit,
            cleanup,
        ];
        let time_ms =
            u64::try_from(now.duration_since(shared.0.epoch).as_millis()).unwrap_or(u64::MAX);
        timing.publish(frame, time_ms, readings, &mut peaks);
    }
    let ready = if marks.active {
        // Each placed row runs from the previous placed mark, so an absent row
        // leaves no stale mark for the next one to absorb.
        let mut previous = marks.start;
        let schedules = array::from_fn(|row| match marks.end_marks[row] {
            EndMark::Placed => {
                let end = marks.ends[row];
                let reading = Reading::Measured(duration_ms(previous, end));
                previous = end;
                reading
            },
            EndMark::Absent => Reading::Unmeasured,
        });
        PendingFrame::Complete {
            frame: marks.frame,
            interval_ms: duration_ms(marks.start, now),
            schedules,
        }
    } else {
        PendingFrame::Warmup
    };
    marks.ready = ready;
    marks.active = true;
    marks.frame = frame_count.0;
    marks.start = now;
    frame_mark.frame = frame_count.0;
}

fn mark_last(mut marks: ResMut<MainMarks>, mut frame_mark: ResMut<MainFrameMark>) {
    let now = Instant::now();
    marks.ends[LAST_ROW] = now;
    frame_mark.last_end = now;
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::thread::ThreadId;

    use bevy_app::App;
    use bevy_ecs::prelude::ResMut;
    use bevy_ecs::prelude::Resource;

    use super::EndMark;
    use super::FrameStart;
    use super::MainMarks;
    use super::RowEnd;
    use crate::FrameTimingPlugin;

    #[derive(Resource, Default)]
    struct MarkThreads(Vec<ThreadId>);

    fn record_thread(mut threads: ResMut<MarkThreads>) { threads.0.push(thread::current().id()); }

    #[test]
    fn every_mark_schedule_runs_on_the_updating_thread() {
        let mut app = App::new();
        app.add_plugins(FrameTimingPlugin);
        app.finish();
        app.cleanup();
        app.init_resource::<MarkThreads>();
        app.add_systems(FrameStart, record_thread);
        let end_marks = app.world().resource::<MainMarks>().end_marks;
        let mut placed = 1;
        for (row, end_mark) in end_marks.into_iter().enumerate() {
            if matches!(end_mark, EndMark::Placed) {
                app.add_systems(RowEnd(row), record_thread);
                placed += 1;
            }
        }

        app.update();
        let threads = &app.world().resource::<MarkThreads>().0;
        assert_eq!(threads.len(), placed, "a mark schedule did not run");
        assert!(
            threads.iter().all(|id| *id == thread::current().id()),
            "a mark ran on a task-pool thread"
        );
    }
}
