//! Per-frame wall-time breakdowns for Bevy's main and render schedules.
//!
//! The reflected [`FrameTiming`] resource is the service surface.
//! Rows keep their last measured sample; main and render rows can overlap.

mod main_thread;
mod render_thread;

use std::array;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Instant;

use bevy_app::App;
use bevy_app::Plugin;
use bevy_ecs::prelude::Resource;
use bevy_ecs::reflect::ReflectResource;
use bevy_reflect::Reflect;

const PEAK_WINDOW_MS: u64 = 5000;

/// Installs the main-schedule and render-stage timing marks.
pub struct FrameTimingPlugin;

impl Plugin for FrameTimingPlugin {
    fn build(&self, app: &mut App) {
        let shared = SharedTiming(Arc::new(CrossThreadTiming::new(Instant::now())));
        app.insert_resource(shared.clone())
            .init_resource::<FrameTiming>()
            .register_type::<SpanSample>()
            .register_type::<SpanTiming>()
            .register_type::<FrameTiming>();
        app.insert_resource(PeakHistory::default());
        main_thread::build(app);
        render_thread::install(app, shared);
    }

    // Every plugin has built by now, so the marks see the final main schedule
    // order: schedules inserted after `Last`, such as BRP's `RemoteLast`, count
    // in the "Last" row, and `StateTransition` is timed only when present.
    fn finish(&self, app: &mut App) { main_thread::install_marks(app); }
}

/// Whether a span has ever been measured, and its last measured value when it has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub enum SpanSample {
    /// No completed record has measured this span.
    #[default]
    NeverMeasured,
    /// The last measured value, retained across records with no reading.
    Measured {
        /// Bevy `FrameCount` of the measured frame.
        frame:          u32,
        /// Duration of that frame, in milliseconds.
        last_ms:        f32,
        /// Exponentially smoothed duration, in milliseconds.
        smoothed_ms:    f32,
        /// Monotonic milliseconds since the timing service started.
        measured_at_ms: u64,
    },
}

/// One span's last measured sample and rolling five-second peak.
#[derive(Clone, Copy, Debug, Default, Reflect)]
pub struct SpanTiming {
    /// The span's last measured sample, or that it has never been measured.
    pub sample:     SpanSample,
    /// Largest duration in the five seconds ending at the last measurement,
    /// retained until this span is measured again.
    pub peak_5s_ms: f32,
}

/// A span's reading within the five seconds ending at a frame record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CurrentSpanReading {
    /// The span was never measured or its last measurement is too old.
    Unavailable,
    /// The span's smoothed duration and rolling peak, in milliseconds.
    Measured {
        /// Exponentially smoothed duration.
        smoothed_ms: f32,
        /// Largest duration in the five-second window.
        peak_5s_ms:  f32,
    },
}

/// Latest complete frame's main and render wall-time breakdown.
///
/// Main rows, the render wait, extract, and `other` partition `frame_time`; a
/// wait or extract with no reading for the record falls into `other`. Render
/// rows describe work running concurrently and must not be added to main rows.
#[derive(Clone, Debug, Default, Reflect, Resource)]
#[reflect(Resource)]
pub struct FrameTiming {
    /// Bevy `FrameCount` of the complete record.
    pub frame:                  u32,
    /// Monotonic milliseconds since the timing service started when this record completed.
    pub published_at_ms:        u64,
    /// Interval from this frame's first mark to the next frame's first mark.
    pub frame_time:             SpanTiming,
    /// The `First` schedule.
    pub first:                  SpanTiming,
    /// The `PreUpdate` schedule.
    pub pre_update:             SpanTiming,
    /// The `StateTransition` schedule, measured only when a states plugin adds it.
    pub state_transition:       SpanTiming,
    /// The `RunFixedMainLoop` schedule.
    pub fixed_update:           SpanTiming,
    /// The `Update` schedule.
    pub update:                 SpanTiming,
    /// The `SpawnScene` schedule.
    pub spawn_scene:            SpanTiming,
    /// The `PostUpdate` schedule.
    pub post_update:            SpanTiming,
    /// The `Last` schedule and every schedule ordered after it.
    pub last:                   SpanTiming,
    /// Main thread time blocked until the render app can be extracted.
    pub waiting_for_render:     SpanTiming,
    /// Main thread work extracting the current frame into the render app.
    pub extract:                SpanTiming,
    /// Frame interval not covered by schedules, render wait, or extraction.
    pub other:                  SpanTiming,
    /// Render thread work applying extracted commands.
    pub extract_commands:       SpanTiming,
    /// Render thread asset preparation.
    pub prepare_assets:         SpanTiming,
    /// Render thread image acquisition, including present or vsync wait.
    pub waiting_for_next_image: SpanTiming,
    /// Remaining render preparation and queue work.
    pub prepare_and_queue:      SpanTiming,
    /// Render graph work before command submission.
    pub render_graph:           SpanTiming,
    /// Render command submission.
    pub submit:                 SpanTiming,
    /// Render cleanup and schedule closeout.
    pub cleanup:                SpanTiming,
}

/// Rows in a [`FrameTiming`] record.
const ROWS: usize = 19;

impl FrameTiming {
    /// Every row in field order, the one order `publish` readings follow.
    const fn spans_mut(&mut self) -> [&mut SpanTiming; ROWS] {
        [
            &mut self.frame_time,
            &mut self.first,
            &mut self.pre_update,
            &mut self.state_transition,
            &mut self.fixed_update,
            &mut self.update,
            &mut self.spawn_scene,
            &mut self.post_update,
            &mut self.last,
            &mut self.waiting_for_render,
            &mut self.extract,
            &mut self.other,
            &mut self.extract_commands,
            &mut self.prepare_assets,
            &mut self.waiting_for_next_image,
            &mut self.prepare_and_queue,
            &mut self.render_graph,
            &mut self.submit,
            &mut self.cleanup,
        ]
    }

    fn publish(
        &mut self,
        frame: u32,
        time_ms: u64,
        readings: [Reading; ROWS],
        peaks: &mut PeakHistory,
    ) {
        self.frame = frame;
        self.published_at_ms = time_ms;
        for ((span, reading), peaks) in self
            .spans_mut()
            .into_iter()
            .zip(readings)
            .zip(&mut peaks.rows)
        {
            if let Reading::Measured(value) = reading {
                span.observe(frame, value, time_ms, peaks);
            }
        }
    }
}

impl SpanTiming {
    /// Returns the reading current at the record's publication time.
    #[must_use]
    pub const fn current_reading(&self, published_at_ms: u64) -> CurrentSpanReading {
        match self.sample {
            SpanSample::Measured {
                smoothed_ms,
                measured_at_ms,
                ..
            } if published_at_ms.saturating_sub(measured_at_ms) <= PEAK_WINDOW_MS => {
                CurrentSpanReading::Measured {
                    smoothed_ms,
                    peak_5s_ms: self.peak_5s_ms,
                }
            },
            SpanSample::NeverMeasured | SpanSample::Measured { .. } => {
                CurrentSpanReading::Unavailable
            },
        }
    }

    fn observe(&mut self, frame: u32, value: f32, time_ms: u64, peaks: &mut PeakDeque) {
        let smoothed_ms = match self.sample {
            SpanSample::Measured {
                smoothed_ms,
                measured_at_ms,
                ..
            } if time_ms.saturating_sub(measured_at_ms) <= PEAK_WINDOW_MS => {
                (value - smoothed_ms).mul_add(0.2, smoothed_ms)
            },
            SpanSample::NeverMeasured | SpanSample::Measured { .. } => value,
        };
        self.sample = SpanSample::Measured {
            frame,
            last_ms: value,
            smoothed_ms,
            measured_at_ms: time_ms,
        };
        self.peak_5s_ms = peaks.observe(time_ms, value);
    }
}

/// One row's value for one record.
#[derive(Clone, Copy)]
enum Reading {
    /// Milliseconds measured for the record.
    Measured(f32),
    /// Nothing measured the row for the record.
    Unmeasured,
}

impl Reading {
    /// The part of the frame interval this reading accounts for.
    const fn accounted_ms(self) -> f32 {
        match self {
            Self::Measured(value) => value,
            Self::Unmeasured => 0.0,
        }
    }

    /// Limits a measured value to `[0, ceiling_ms]`.
    const fn clamped(self, ceiling_ms: f32) -> Self {
        match self {
            Self::Measured(value) => Self::Measured(value.clamp(0.0, ceiling_ms.max(0.0))),
            Self::Unmeasured => Self::Unmeasured,
        }
    }

    /// Cross-thread encoding; NaN never comes from a duration, so it stands for
    /// `Unmeasured`.
    const fn to_bits(self) -> u32 {
        match self {
            Self::Measured(value) => value.to_bits(),
            Self::Unmeasured => f32::NAN.to_bits(),
        }
    }

    const fn from_bits(bits: u32) -> Self {
        let value = f32::from_bits(bits);
        if value.is_nan() {
            Self::Unmeasured
        } else {
            Self::Measured(value)
        }
    }
}

#[derive(Resource)]
struct PeakHistory {
    rows: [PeakDeque; ROWS],
}

impl Default for PeakHistory {
    fn default() -> Self {
        Self {
            rows: array::from_fn(|_| PeakDeque::default()),
        }
    }
}

#[derive(Clone, Copy)]
struct PeakEntry {
    time_ms: u64,
    value:   f32,
}

struct PeakDeque {
    entries: Vec<PeakEntry>,
    head:    usize,
    len:     usize,
}

impl Default for PeakDeque {
    fn default() -> Self {
        Self {
            entries: vec![
                PeakEntry {
                    time_ms: 0,
                    value:   0.0,
                };
                5002
            ],
            head:    0,
            len:     0,
        }
    }
}

impl PeakDeque {
    fn observe(&mut self, time_ms: u64, value: f32) -> f32 {
        // Keep one maximum per millisecond. The fixed monotone ring covers
        // every millisecond in the inclusive 5 s window at any frame rate.
        let capacity = self.entries.len();
        if self.len > 0 {
            let tail = (self.head + self.len - 1) % capacity;
            if self.entries[tail].time_ms == time_ms && self.entries[tail].value >= value {
                while self.len > 1
                    && self.entries[self.head]
                        .time_ms
                        .saturating_add(PEAK_WINDOW_MS)
                        < time_ms
                {
                    self.head = (self.head + 1) % capacity;
                    self.len -= 1;
                }
                return self.entries[self.head].value;
            }
        }
        while self.len > 0 {
            let tail = (self.head + self.len - 1) % capacity;
            if self.entries[tail].value > value {
                break;
            }
            self.len -= 1;
        }
        if self.len == capacity {
            self.head = (self.head + 1) % capacity;
            self.len -= 1;
        }
        let tail = (self.head + self.len) % capacity;
        self.entries[tail] = PeakEntry { time_ms, value };
        self.len += 1;
        while self.len > 1
            && self.entries[self.head]
                .time_ms
                .saturating_add(PEAK_WINDOW_MS)
                < time_ms
        {
            self.head = (self.head + 1) % capacity;
            self.len -= 1;
        }
        self.entries[self.head].value
    }
}

/// Render stages timed on the render thread, in `FrameTiming` order.
const RENDER_ROWS: usize = 7;

#[derive(Resource, Clone)]
struct SharedTiming(Arc<CrossThreadTiming>);

/// Readings handed between threads, three frames deep and keyed by frame so a
/// reader can tell a reading for its record from a stale one.
struct CrossThreadTiming {
    epoch:            Instant,
    extract_frame:    AtomicU32,
    wait_frame:       [AtomicU32; 3],
    wait_ms:          [AtomicU32; 3],
    extraction_frame: [AtomicU32; 3],
    extraction_ms:    [AtomicU32; 3],
    render_frame:     [AtomicU32; 3],
    render_ms:        [[AtomicU32; RENDER_ROWS]; 3],
}

const fn slot(frame: u32) -> usize { frame as usize % 3 }

impl CrossThreadTiming {
    fn new(epoch: Instant) -> Self {
        Self {
            epoch,
            extract_frame: AtomicU32::new(u32::MAX),
            wait_frame: array::from_fn(|_| AtomicU32::new(u32::MAX)),
            wait_ms: array::from_fn(|_| AtomicU32::new(0)),
            extraction_frame: array::from_fn(|_| AtomicU32::new(u32::MAX)),
            extraction_ms: array::from_fn(|_| AtomicU32::new(0)),
            render_frame: array::from_fn(|_| AtomicU32::new(u32::MAX)),
            render_ms: array::from_fn(|_| array::from_fn(|_| AtomicU32::new(0))),
        }
    }

    fn publish_render(&self, frame: u32, readings: [Reading; RENDER_ROWS]) {
        let slot = slot(frame);
        for (cell, reading) in self.render_ms[slot].iter().zip(readings) {
            cell.store(reading.to_bits(), Ordering::Relaxed);
        }
        self.render_frame[slot].store(frame, Ordering::Release);
    }

    fn render_for(&self, frame: u32) -> [Reading; RENDER_ROWS] {
        let slot = slot(frame);
        if self.render_frame[slot].load(Ordering::Acquire) != frame {
            return [Reading::Unmeasured; RENDER_ROWS];
        }
        array::from_fn(|row| Reading::from_bits(self.render_ms[slot][row].load(Ordering::Relaxed)))
    }

    fn waiting_for_render(&self, frame: u32) -> Reading {
        let slot = slot(frame);
        reading_for(frame, &self.wait_frame[slot], &self.wait_ms[slot])
    }

    fn extraction(&self, frame: u32) -> Reading {
        let slot = slot(frame);
        reading_for(
            frame,
            &self.extraction_frame[slot],
            &self.extraction_ms[slot],
        )
    }
}

fn reading_for(frame: u32, measured_frame: &AtomicU32, value: &AtomicU32) -> Reading {
    if measured_frame.load(Ordering::Acquire) != frame {
        return Reading::Unmeasured;
    }
    Reading::Measured(f32::from_bits(value.load(Ordering::Relaxed)))
}

fn duration_ms(start: Instant, end: Instant) -> f32 {
    end.saturating_duration_since(start).as_secs_f32() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::PeakDeque;

    #[test]
    fn five_second_peak_survives_many_samples_per_millisecond() {
        let mut peak = PeakDeque::default();
        for millisecond in 0_u16..=5000 {
            let value = f32::from(5001 - millisecond);
            for _ in 0..4 {
                assert_eq!(peak.observe(u64::from(millisecond), value), 5001.0);
            }
        }
        assert_eq!(peak.observe(5001, 0.0), 5000.0);
    }
}
