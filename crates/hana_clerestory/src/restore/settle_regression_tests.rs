//! Standing proof that a window already at its target settles as a match.
//!
//! `check_restore_settling` ends an attempt two ways: `WindowRestored` once the observed window has
//! held the dispatched geometry for `SETTLE_STABILITY_SECS`, and `WindowRestoreMismatch` once it is
//! stable somewhere else or the `SETTLE_TIMEOUT_SECS` deadline expires. A window already at the
//! geometry it was dispatched to must take the first of those, on the frame after the one that
//! first observed it, and must never reach the deadline.
//!
//! The conformance walk was meant to carry that claim and could not. Its virtual clock clamped a
//! one-millisecond real step and scaled the result by a relative speed large enough that a single
//! frame advanced the settle past `SETTLE_TIMEOUT_SECS`, so every restore in the walk expired on
//! its first settle frame — before a second observation existed to compare against — and ended
//! through `emit_settle_mismatch` with every expected/actual pair identical. `finish_with_readback`
//! reports that to the kernel as a success, so nothing downstream distinguished it from a real
//! match, and the walk was green while proving the opposite of what it claimed.
//!
//! This test is what keeps the claim from depending on any walk's clock. It drives one restore
//! through the production plugin set on a clock stepped no faster than the settle can observe, and
//! reads the two events. The frame budget is deliberately smaller than the deadline needs, so a
//! pass cannot be bought by the timeout path: a `WindowRestored` inside the budget is a settle that
//! resolved on stability. What it guards against is a clock a fixture steps too coarsely, and any
//! future change that makes the settle observation churn between frames — either one turns a
//! matching window into a deadline mismatch reported to the kernel as a success.

use std::time::Duration;

use bevy::prelude::App;
use bevy::prelude::Entity;
use bevy::prelude::On;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::UVec2;
use bevy::time::Time;
use bevy::time::TimeUpdateStrategy;
use bevy::time::Virtual;
use bevy::window::OnMonitor;

use super::InjectedWinitWindows;
use super::target_position::TargetPosition;
use super::target_position::WindowSettleProgress;
use crate::events::WindowRestoreMismatch;
use crate::events::WindowRestored;
use crate::monitors::CurrentMonitor;
use crate::monitors::Monitors;
use crate::tests;
use crate::tests::ApplyingWindowRole;

/// Virtual time one frame of this test carries.
///
/// Longer than `SETTLE_STABILITY_SECS` so a motionless window becomes stable on the frame after the
/// one that first observed it, and a small fraction of `SETTLE_TIMEOUT_SECS` so the deadline is
/// still pending for every frame the budget below allows.
///
/// The restore pipeline reads `Time`, which `Time<Virtual>` feeds by clamping the raw step to its
/// own maximum and then scaling by its relative speed. `TimeUpdateStrategy::ManualDuration` sets
/// the raw step to this value and the maximum is set to it as well, so the clamp is a no-op and the
/// default relative speed of one leaves the frame worth exactly this. Both are set rather than one,
/// because a raw step alone would still be capped by whatever maximum the clock happened to carry.
/// The order is the trap: a large relative speed multiplies *after* the clamp, so a maximum of this
/// size does not bound the virtual frame at all once the speed is raised.
const REGRESSION_FRAME: Duration = Duration::from_millis(250);

/// Frames the restore is given to resolve.
///
/// `SETTLE_TIMEOUT_SECS` is two seconds, which is eight [`REGRESSION_FRAME`]s, so no settle that
/// ends inside this budget can have ended on the expired deadline. Two frames of settling are what
/// a matching window actually needs — one to record the observation and one to find it unchanged —
/// and the remainder covers the frames the pipeline spends reaching the window before settle
/// starts.
const SETTLE_RESOLUTION_FRAME_BUDGET: usize = 6;

/// How the restore under test ended, counted per event rather than latched.
///
/// Counting rather than keeping the last event is what makes "and no mismatch" assertable: the two
/// events are the only two endings, and a run that produced both would be a driver ending one
/// attempt twice. Every mismatch is kept whole because its expected/actual pairs are the only
/// explanation available when a restore ends the wrong way.
#[derive(Resource, Debug, Default)]
struct SettleResolutionTally {
    /// `WindowRestored` triggers seen — the window reached the geometry it was dispatched to.
    matched:  usize,
    /// Every `WindowRestoreMismatch` trigger seen, as reported.
    diverged: Vec<WindowRestoreMismatch>,
}

impl SettleResolutionTally {
    /// Whether the restore has ended, either way.
    fn resolved(&self) -> bool { self.matched > 0 || !self.diverged.is_empty() }
}

/// How far the restore under test got before it ended or the frame budget ran out.
///
/// A restore that never started settling proves nothing about a settle that resolves the wrong way,
/// and the two would otherwise fail this test identically: no `WindowRestored`, no mismatch. Naming
/// the reach is what keeps a fixture whose window never got placed from reading as the defect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettleReach {
    /// No frame observed the window in `WindowSettleProgress::Settling`.
    NeverStartedSettling,
    /// At least one frame observed the window settling.
    StartedSettling,
}

fn record_settle_match(_: On<WindowRestored>, mut tally: ResMut<SettleResolutionTally>) {
    tally.matched += 1;
}

fn record_settle_divergence(
    mismatch: On<WindowRestoreMismatch>,
    mut tally: ResMut<SettleResolutionTally>,
) {
    tally.diverged.push(WindowRestoreMismatch::clone(&mismatch));
}

/// Supply the two facts the restore pipeline needs that a dispatched attempt does not carry.
///
/// `prepare_driver_restore_targets` skips a window whose `OnMonitor` does not name the same live
/// display its `CurrentMonitor` describes — `exact_monitor_association` draws that line — and
/// `place_window_at_saved_geometry` skips a window with no native window behind it. The fixture
/// stops at a dispatched attempt, and this test has no winit, so both are the test's to supply.
/// Zero decoration is the whole native fact the placement path reads, and the association is the
/// one the shipped conformance subject authors for the windows it spawns.
fn complete_the_restore_fixture(app: &mut App, window: Entity) -> Result<(), String> {
    app.world_mut().init_resource::<InjectedWinitWindows>();
    app.world_mut()
        .resource_mut::<InjectedWinitWindows>()
        .insert(window, UVec2::ZERO);

    let descriptor = app
        .world()
        .get::<CurrentMonitor>(window)
        .ok_or_else(|| String::from("the dispatched window carries no current monitor"))?
        .descriptor;
    let monitor = app
        .world()
        .resource::<Monitors>()
        .iter()
        .find(|live| *live.descriptor == descriptor)
        .map(|live| live.entity)
        .ok_or_else(|| {
            String::from("no live display matches the descriptor the window reports occupying")
        })?;
    app.world_mut()
        .entity_mut(window)
        .insert(OnMonitor(monitor));
    Ok(())
}

/// Step the clock so one frame is worth exactly [`REGRESSION_FRAME`] of virtual time.
fn step_the_clock_one_regression_frame_per_update(app: &mut App) {
    app.insert_resource(TimeUpdateStrategy::ManualDuration(REGRESSION_FRAME));
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .set_max_delta(REGRESSION_FRAME);
}

/// Run the applying restore until it ends or the frame budget runs out, reporting how far it got.
fn resolve_one_restore(app: &mut App, window: Entity) -> SettleReach {
    let mut reach = SettleReach::NeverStartedSettling;
    for _ in 0..SETTLE_RESOLUTION_FRAME_BUDGET {
        app.update();
        if app
            .world()
            .get::<TargetPosition>(window)
            .is_some_and(|target_position| {
                matches!(
                    target_position.window_settle_progress,
                    WindowSettleProgress::Settling(_)
                )
            })
        {
            reach = SettleReach::StartedSettling;
        }
        if app.world().resource::<SettleResolutionTally>().resolved() {
            break;
        }
    }
    reach
}

/// A window already at its dispatched geometry settles as a match, not at the deadline.
#[test]
fn a_window_that_never_moves_settles_as_a_match() -> Result<(), String> {
    let ApplyingWindowRole {
        mut app, window, ..
    } = tests::applying_window_role()?;

    complete_the_restore_fixture(&mut app, window)?;
    step_the_clock_one_regression_frame_per_update(&mut app);
    app.init_resource::<SettleResolutionTally>()
        .add_observer(record_settle_match)
        .add_observer(record_settle_divergence);

    let reach = resolve_one_restore(&mut app, window);

    let tally = app.world().resource::<SettleResolutionTally>();
    assert_eq!(
        reach,
        SettleReach::StartedSettling,
        "the fixture's restore never reached settle, so this run says nothing about how a settle \
         resolves",
    );
    assert!(
        tally.diverged.is_empty(),
        "a window left at its dispatched geometry reported a restore mismatch: {:?}",
        tally.diverged,
    );
    assert_eq!(
        tally.matched, 1,
        "a window left at its dispatched geometry did not settle as a match within \
         {SETTLE_RESOLUTION_FRAME_BUDGET} frames",
    );
    Ok(())
}
