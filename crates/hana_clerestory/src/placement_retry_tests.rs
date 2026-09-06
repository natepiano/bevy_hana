//! An abandoned placement is retried when the display it was waiting for returns.
//!
//! A display that leaves and comes back is the normal case for a projector rig, and the window
//! whose placement was abandoned for it is the one an operator can least afford to lose: every
//! main-world component reads correct while the output stays black. These tests pin the one path
//! that used to end terminally — the `ProjectedRolePlacementStatus::PlacementImpossible` arm of
//! `handle_kernel_status_at_deadline`.
//!
//! What distinguishes the four cases is which display arrives and whether anything durable names
//! the one the window is waiting for. `resume_placement_when_wanted_display_returns` matches an
//! arrival's evidence against the window's `WantedDisplay`, so an arrival that classifies to a
//! different `DeviceKey` must not move it, an arrival carrying no durable key at all must not move
//! it however closely its evidence resembles the wanted display's, and a window whose wanted
//! display is `Unnamed` can never be matched by any arrival.
//!
//! # Where each test does its work
//!
//! The retried case is asserted at the end, on where the window ended up: its saved geometry, and
//! the monitor entity it is sitting on. Geometry alone would not settle it, because a fallback fit
//! can land a window on the same numbers; the monitor entity is what separates "placed back on the
//! display it was saved to" from "fitted onto whatever was available".
//!
//! The three refusing cases are asserted on the frame the arrival lands, before any deadline is
//! advanced. That is the only frame where a wrongly matched window is distinguishable at all: an
//! observer that matched an arrival it should have refused re-arms the window, runs the deadline
//! against a display that cannot hold it, and abandons it a second time, so by the end of the test
//! a wrongly matched window and an untouched one read the same. Each is then asserted again after a
//! full timeout, so "still abandoned" means "still abandoned after a retry had every chance to
//! complete" rather than "asserted one frame too early".
//!
//! The role behind the window is read on both of those frames too, and must not have established.
//! The kernel does not re-attempt a role on its own; the observer restarting placement is the only
//! thing that would, so an established role in a refusing case is the transition itself, showing up
//! one layer down from the window.

use std::time::Duration;

use bevy::prelude::App;
use bevy::prelude::Entity;
use bevy::prelude::Window;
use bevy::prelude::WindowPosition;
use hana_rigging::prelude::RoleStatusView;

use crate::WindowRevealDisposition;
use crate::constants::EXACT_DISPLAY_WAIT_TIMEOUT_SECS;
use crate::monitors::CurrentMonitorEntity;
use crate::visibility::PlacementAbandoned;
use crate::visibility::tests;
use crate::visibility::tests::AbandonedPlacementScript;
use crate::visibility::tests::RetryDisplayArrival;
use crate::visibility::tests::SAVED_LOGICAL_HEIGHT;
use crate::visibility::tests::SAVED_LOGICAL_WIDTH;
use crate::visibility::tests::SAVED_WINDOW_OFFSET;

/// Where a window sits, as an operator would see it.
///
/// Position and size are read together because either alone can match by accident: a window left
/// untouched and a window placed onto a display of the same geometry differ in both or in neither.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WindowPlacementReading {
    position:        WindowPosition,
    physical_width:  u32,
    physical_height: u32,
}

impl WindowPlacementReading {
    /// The geometry a window placed on its own saved display must be showing.
    const fn saved() -> Self {
        Self {
            position:        WindowPosition::At(SAVED_WINDOW_OFFSET),
            physical_width:  SAVED_LOGICAL_WIDTH,
            physical_height: SAVED_LOGICAL_HEIGHT,
        }
    }
}

fn read_window_placement(app: &App, window: Entity) -> Result<WindowPlacementReading, String> {
    let window = app
        .world()
        .get::<Window>(window)
        .ok_or_else(|| String::from("the managed window disappeared"))?;
    Ok(WindowPlacementReading {
        position:        window.position,
        physical_width:  window.resolution.physical_width(),
        physical_height: window.resolution.physical_height(),
    })
}

/// How far the kernel got with the role behind one window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RolePlacementReading {
    /// The role established on a display, which is what the wanted display's return makes possible.
    Established,
    /// No display has satisfied the role: it is still waiting, applying, or stopped.
    Unsatisfied,
}

fn read_role_placement(app: &App, window: Entity) -> Result<RolePlacementReading, String> {
    Ok(match tests::production_window_status(app, window)?.view() {
        RoleStatusView::Established { .. } => RolePlacementReading::Established,
        RoleStatusView::Waiting(_)
        | RoleStatusView::Applying { .. }
        | RoleStatusView::Stopped(_) => RolePlacementReading::Unsatisfied,
    })
}

/// The app time an abandoned window has been waiting since, or `None` once it is no longer waiting.
///
/// The instant, not just the presence, is what separates a refused arrival from a wrongly matched
/// one. A wrong match consumes `PlacementAbandoned`, re-arms the wait, and abandons the window
/// again at a later instant — so a window carrying the very same abandoned state it was given is
/// the proof that no transition happened, where "some abandoned state is present" is not.
fn abandoned_since(app: &App, window: Entity) -> Option<Duration> {
    app.world()
        .get::<PlacementAbandoned>(window)
        .map(PlacementAbandoned::since)
}

/// Assert that an arrival did not restart placement.
///
/// All three facts are needed, because each is one face of the same transition and each can read
/// innocently alone in the frames around a deadline. A re-armed window has given up its abandoned
/// state and holds a fresh `SavedDisplayRevealWait`; the role behind it reaches `Established` only
/// if something re-attempted it, and the kernel never re-attempts on its own — the observer
/// restarting placement is the only thing that would, which is exactly what these cases deny.
fn assert_placement_not_restarted(
    app: &App,
    window: Entity,
    since_when_abandoned: Option<Duration>,
    when: &str,
) -> Result<(), String> {
    assert_ne!(
        read_role_placement(app, window)?,
        RolePlacementReading::Established,
        "the role must not have established {when}; the kernel does not re-attempt on its own, so \
         an established role means placement was restarted"
    );
    assert_eq!(
        abandoned_since(app, window),
        since_when_abandoned,
        "the window must still carry the abandoned state it was given {when} — a missing or later \
         one means the arrival was matched, retried, and abandoned again"
    );
    assert!(
        !tests::has_reveal_wait(app, window),
        "no placement wait may be armed {when}; an armed wait is a retry already under way"
    );
    Ok(())
}

/// Drive an arrival that must be refused, and assert nothing restarted on either side of a timeout.
fn assert_arrival_refused(
    app: &mut App,
    window: Entity,
    arrival: RetryDisplayArrival,
    what_arrived: &str,
) -> Result<(), String> {
    let since_when_abandoned = abandoned_since(app, window);
    assert!(
        since_when_abandoned.is_some(),
        "the window must be abandoned before {what_arrived} arrives, or the test proves nothing"
    );

    tests::script_display_arrival(app, arrival)?;
    assert_placement_not_restarted(
        app,
        window,
        since_when_abandoned,
        &format!("on the frame {what_arrived} arrives"),
    )?;

    settle_retry(app);
    assert_placement_not_restarted(
        app,
        window,
        since_when_abandoned,
        &format!("a full placement timeout after {what_arrived} arrived"),
    )?;
    Ok(())
}

/// Run the re-entry, the deadline, and placement to their end after a display returns.
///
/// A matched arrival is not a placement. `reenter_abandoned_placement` hides the window again
/// behind a fresh `SavedDisplayRevealWait` and removes its `WindowRevealDisposition`, which starts
/// a first attempt's deadline from the top; only after that deadline has run does the window reach
/// its saved geometry.
fn settle_retry(app: &mut App) { tests::advance(app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS); }

#[test]
fn an_abandoned_window_is_placed_on_its_display_when_that_display_returns() -> Result<(), String> {
    let (mut app, primary_window) =
        tests::abandoned_placement_app(AbandonedPlacementScript::WantedDisplayDeparted)?;
    assert!(
        abandoned_since(&app, primary_window).is_some(),
        "the impossible-placement arm must leave the window abandoned before any arrival"
    );

    tests::script_display_arrival(&mut app, RetryDisplayArrival::WantedDisplay)?;
    settle_retry(&mut app);

    assert_eq!(
        abandoned_since(&app, primary_window),
        None,
        "the matched arrival must consume the abandoned state, not leave it on the entity"
    );
    assert_ne!(
        app.world().get::<WindowRevealDisposition>(primary_window),
        Some(&WindowRevealDisposition::PlacementAbandoned),
        "a window that was placed again cannot still be recorded as abandoned"
    );
    assert_eq!(
        read_role_placement(&app, primary_window)?,
        RolePlacementReading::Established,
        "the wanted display came back, so the role behind the window must have established on it"
    );
    assert_eq!(
        read_window_placement(&app, primary_window)?,
        WindowPlacementReading::saved(),
        "the window must be showing the geometry it was saved with"
    );
    assert_eq!(
        app.world()
            .get::<CurrentMonitorEntity>(primary_window)
            .map(|monitor| monitor.entity()),
        Some(tests::wanted_display_monitor(&app)?),
        "the window must be sitting on the display it was saved to; the saved geometry alone does \
         not say that, because a fallback fit can reproduce those same numbers"
    );
    Ok(())
}

#[test]
fn an_unrelated_display_arriving_leaves_an_abandoned_window_abandoned() -> Result<(), String> {
    let (mut app, primary_window) =
        tests::abandoned_placement_app(AbandonedPlacementScript::WantedDisplayDeparted)?;

    // This display's evidence classifies to a different `DeviceKey` than the one the window was
    // saved against, which is the whole content of the test: matching on arrival rather than on
    // identity would restart the window on whatever hardware happened to be plugged in next.
    assert_arrival_refused(
        &mut app,
        primary_window,
        RetryDisplayArrival::UnrelatedDisplay,
        "a display the window was never saved against",
    )
}

#[test]
fn a_display_carrying_no_durable_key_never_matches_an_abandoned_window() -> Result<(), String> {
    let (mut app, primary_window) =
        tests::abandoned_placement_app(AbandonedPlacementScript::WantedDisplayDeparted)?;

    // This arrival's evidence classifies to `DisplayKeyClassification::MatchEvidenceOnly`: it
    // carries no durable serial, so nothing about it can establish which display it is. Treating
    // that as a match would restart the window on whichever unidentifiable display was plugged in,
    // which is the failure this arm of `WantedDisplay::returned_as` exists to prevent.
    assert_arrival_refused(
        &mut app,
        primary_window,
        RetryDisplayArrival::EvidenceOnlyLookalike,
        "a display whose evidence names no device",
    )
}

#[test]
fn a_window_abandoned_with_no_usable_key_stays_abandoned_through_any_arrival() -> Result<(), String>
{
    let (mut app, primary_window) =
        tests::abandoned_placement_app(AbandonedPlacementScript::NoUsableKey)?;

    // Nothing durable names the display this window was placed on, so its `WantedDisplay` is
    // `Unnamed` and no arrival can establish that this window's display returned — including
    // the display it actually opened on. The binding is untouched by this script; only the
    // persisted target is cleared, which is the sole thing `WantedDisplay::for_role` reads.
    //
    // Both arrivals are driven to show that the count of arrivals is not what is being tested: no
    // number of them can match a want that names nothing.
    assert_arrival_refused(
        &mut app,
        primary_window,
        RetryDisplayArrival::WantedDisplay,
        "the display the window opened on",
    )?;

    assert_arrival_refused(
        &mut app,
        primary_window,
        RetryDisplayArrival::UnrelatedDisplay,
        "a second, unrelated display",
    )
}
