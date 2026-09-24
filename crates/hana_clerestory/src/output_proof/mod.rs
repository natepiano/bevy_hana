//! Operating-system confirmation for Clerestory output windows.
//!
//! A main-world window component can say only what Clerestory asked winit to do. The operating
//! system is the hardware for a window, so this module takes the last observation available before
//! the projector: whether `AppKit` reports that the native window is on a screen and visible. That
//! report does not identify the screen and does not prove that the application's pixels were
//! drawn. No presented-frame callback is reachable through winit or wgpu today;
//! `CAMetalDrawable.addPresentedHandler` is the per-frame signal to replace this poll with when a
//! hook into the surface becomes available.
//!
//! The poll runs wherever the signal is provided: on macOS, whose `native` backend reads `AppKit`,
//! and in tests, which script the reading. On other production platforms the `native` backend
//! declares the signal absent, so the poll is never installed and every output window explicitly
//! carries `OutputProof::NoSignalAvailable` instead of an unconfirmed proof that promises evidence
//! which can never arrive.

#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(not(target_os = "macos"), path = "unsupported.rs")]
mod native;

#[cfg(test)]
use std::collections::HashMap;
use std::time::Duration;

use bevy::app::App;
use bevy::app::PostUpdate;
use bevy::ecs::system::Local;
use bevy::ecs::system::NonSendMarker;
use bevy::prelude::Entity;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Query;
use bevy::prelude::Res;
#[cfg(test)]
use bevy::prelude::ResMut;
#[cfg(test)]
use bevy::prelude::Resource;
use bevy::time::Time;
use bevy::time::Virtual;
use hana_rigging::prelude::ConfirmationSignal;
use hana_rigging::prelude::OutputConfirmation;
use hana_rigging::prelude::OutputConfirmationSource;
use hana_rigging::prelude::OutputProofPlugin;
use hana_rigging::prelude::OutputProofSystems;

#[cfg(test)]
use crate::restore::InjectedWinitWindows;

/// Confirms an output window from the operating system's own visibility report.
///
/// The OS is the hardware for a window, and its visibility report is the last observable before
/// the projector: this source confirms that the window is on some screen and that `AppKit` reports
/// it visible. It does not say which screen the window is on, and it does not say that pixels were
/// drawn. `AppKit` also clears visibility when the app is hidden, its Space is inactive, the
/// screen is locked, or the display sleeps; in every one of those cases the output is not being
/// shown, so `Unconfirmed` is the correct reading. A window fitted onto a fallback display reads
/// `Confirmed` exactly like one on its saved display. During a retry on a display's return, the
/// window hides for up to `SETTLE_TIMEOUT_SECS`, longer than the cadence, so the proof dips to
/// `Unconfirmed` and returns to `Confirmed` whether the retry landed on the wanted display or timed
/// out onto the fallback. Which display a window sits on is `CurrentMonitorEntity` and
/// `WindowRevealDisposition`, not this proof.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OnScreenConfirmation;

impl OutputConfirmation for OnScreenConfirmation {
    const SIGNAL: ConfirmationSignal = if cfg!(test) {
        ConfirmationSignal::Provided
    } else {
        native::SIGNAL
    };

    fn cadence(&self) -> Duration { Duration::from_secs(1) }

    fn confirm(&mut self, _: &Time<Virtual>) {}
}

/// Install the complete operating-system proof mechanism before any output role is inserted.
///
/// The confirmation source's insertion hook requires the aging plugin receipt immediately, while
/// tests also need the same scripted boundary and ordered production poll as the main plugin.
/// Keeping all three registrations here prevents a manual `RiggingPlugin` fixture from silently
/// constructing only part of the mechanism.
pub(crate) fn register_window_output_proof(app: &mut App) {
    app.add_plugins(OutputProofPlugin::<OnScreenConfirmation>::new());
    #[cfg(test)]
    app.init_resource::<ScriptedWindowOnScreenReadings>();
    if matches!(OnScreenConfirmation::SIGNAL, ConfirmationSignal::Provided) {
        app.add_systems(
            PostUpdate,
            poll_windows_on_screen.before(OutputProofSystems::Age),
        );
    }
}

/// What `AppKit` reported about one native window's relationship to the visible desktop.
///
/// These are `AppKit` occlusion readings, not claims about drawn pixels.
/// `NSWindowOcclusionStateVisible` means `AppKit` counts some part of the window's bounds as
/// visible; even a completely transparent window can carry that state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(any(target_os = "macos", test)),
    allow(
        dead_code,
        reason = "this production platform has no window-output poll"
    )
)]
pub(crate) enum WindowOnScreenReading {
    /// `AppKit` reports that some part of the window's bounds is visible.
    ReportedVisible,
    /// `AppKit` reports that no part of the window's bounds is visible.
    ReportedOccluded,
    /// `AppKit` reports that the window is not currently associated with any screen.
    OnNoScreen,
}

/// One complete attempt to observe whether an output window is on screen.
///
/// Native-window absence and scripted-evidence absence are named separately so neither can be
/// mistaken for `AppKit` affirmatively reporting [`WindowOnScreenReading::OnNoScreen`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(any(target_os = "macos", test)),
    allow(
        dead_code,
        reason = "this production platform has no window-output poll"
    )
)]
enum WindowOnScreenObservation {
    /// The OS, or the test double standing in for it, produced this reading.
    Reading(WindowOnScreenReading),
    /// Winit has not created the native window for this entity yet.
    NoNativeWindowYet,
    /// The test double has no reading scripted for this entity.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "only the test double can omit a scripted reading")
    )]
    NoScriptedReading,
}

/// The earliest app time at which the next operating-system observation may be taken.
///
/// Starting at zero deliberately makes a newly installed poll read immediately. Every completed
/// poll moves the deadline by half the source cadence, which detects a stopped report in time for
/// the proof to demote within one full cadence without asking `AppKit` on every frame.
#[derive(Default)]
struct NextWindowOnScreenReadingAt(Duration);

impl NextWindowOnScreenReadingAt {
    fn is_due(&self, now: Duration) -> bool { now >= self.0 }

    const fn postpone_from(&mut self, now: Duration, interval: Duration) {
        self.0 = now.saturating_add(interval);
    }
}

/// Per-window operating-system readings supplied by headless tests.
///
/// The resource replaces only the winit-to-`AppKit` boundary. Counting observations here lets
/// tests pin the production poll's half-cadence rate without exposing its deadline bookkeeping.
#[cfg(test)]
#[derive(Default, Resource)]
pub(crate) struct ScriptedWindowOnScreenReadings {
    readings:       HashMap<Entity, WindowOnScreenReading>,
    readings_taken: usize,
}

#[cfg(test)]
impl ScriptedWindowOnScreenReadings {
    /// Supply or replace the next persistent OS reading for `window`.
    pub(crate) fn script(&mut self, window: Entity, reading: WindowOnScreenReading) {
        self.readings.insert(window, reading);
    }

    /// Count how many window observations the production poll has requested from this resource.
    pub(crate) const fn readings_taken(&self) -> usize { self.readings_taken }

    fn observe(&mut self, window: Entity) -> WindowOnScreenObservation {
        self.readings_taken = self.readings_taken.saturating_add(1);
        self.readings
            .get(&window)
            .copied()
            .map_or(WindowOnScreenObservation::NoScriptedReading, |reading| {
                WindowOnScreenObservation::Reading(reading)
            })
    }
}

/// Confirm every output window that the operating system currently reports on screen and visible.
///
/// The query is over confirmation sources rather than general windows: a
/// `WindowRiggingRole` requires this source for exactly its managed lifetime, so the component is
/// the presenter's declaration that this operating-system reading belongs to it.
fn poll_windows_on_screen(
    time: Res<Time<Virtual>>,
    mut next_reading_at: Local<NextWindowOnScreenReadingAt>,
    mut output_windows: Query<(Entity, &mut OutputConfirmationSource<OnScreenConfirmation>)>,
    _: NonSendMarker,
    #[cfg(test)] injected_windows: Option<Res<InjectedWinitWindows>>,
    #[cfg(test)] mut scripted_readings: ResMut<ScriptedWindowOnScreenReadings>,
) {
    let now = time.elapsed();
    if !next_reading_at.is_due(now) {
        return;
    }
    next_reading_at.postpone_from(now, OnScreenConfirmation.cadence() / 2);

    for (window, mut confirmation) in &mut output_windows {
        // An injected winit resource declares a headless test boundary. Its presence stands native
        // lookup down wholesale; whether evidence exists for this particular entity belongs to the
        // scripted reading and becomes `NoScriptedReading`, never `OnNoScreen`.
        #[cfg(test)]
        let observation = if injected_windows.is_some() {
            scripted_readings.observe(window)
        } else {
            native::observe_native_window_on_screen(window)
        };
        #[cfg(not(test))]
        let observation = native::observe_native_window_on_screen(window);
        if matches!(
            observation,
            WindowOnScreenObservation::Reading(WindowOnScreenReading::ReportedVisible)
        ) {
            confirmation.confirm(&time);
        }
    }
}
