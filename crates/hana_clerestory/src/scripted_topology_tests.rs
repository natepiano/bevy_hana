//! The display reporter keeps completing scans when a consumer spawns monitor entities of its own.
//!
//! [`DisplayTestAdapter`] is the platform for an app that has no winit: it says which displays are
//! attached and what identity material each publishes. A consumer wiring it — `hana`'s screen
//! presentation and startup-lifecycle fixtures are the ones this crate has — adds the adapter, adds
//! `WindowManagerPlugin`, and then spawns [`Monitor`] entities of its own for the displays *its*
//! fixture is about. Those entities are ordinary Bevy monitor topology; they are not displays the
//! scripted platform attached, and the display reporter must go on completing its scans beside
//! them.
//!
//! # The regression
//!
//! `fresh_display_evidence` reads the scripted current-display list against the observed topology.
//! While the adapter published that observation itself the two always agreed, and an unscripted
//! adapter simply produced an empty pair. Once the production monitor scan took ownership of the
//! observation it began enumerating *every* [`Monitor`] entity in the world, including a
//! consumer's, so the observation became permanently non-empty while the scripted current-display
//! list stayed empty — and the reporter answered every scan with `DeviceScan::Failed`. Clerestory's
//! display reporter is registered as required, so nothing waiting on a first complete device set
//! ever moves again. That is the whole stall, and it surfaces far away from here as batch timeouts
//! in the consuming app, with no panic and no failed assertion anywhere near the cause.
//!
//! # What these tests assert, and why it is the reporter rather than the topology
//!
//! Both read the production display reporter: whether it **kept** completing scans once the
//! consumer's monitor entity was in the world, and which displays those scans reported. The
//! insistence on *kept* is what makes the assertion mean anything, and it was learned the hard
//! way. The reporter completes its startup scan before any consumer entity exists, so
//! `completed_runs > 0` on a settled app is
//! satisfied by that first scan and stays true however comprehensively every later scan fails —
//! an assertion phrased that way is green on the broken tree. What the regression actually denies
//! is the *next* scan: the one a topology change requests after the consumer's monitor has joined
//! the observation. So each test takes a baseline count with the platform alone, spawns the
//! consumer's monitor, and requires the count to advance past that baseline.
//!
//! The second test carries a further guarantee that is easy to miss: not merely that the scripted
//! display is reported, but that it is reported by the FIRST completed scan. Agreement between the
//! scripted current-display list and the observed topology is not enough on its own if the two are
//! reached on different frames — a scan taken after the adapter has published its list but before
//! the production topology scan has seen its monitor entities completes cleanly and reports no
//! display at all. A consumer that advances its reporter once and proceeds, as `hana`'s fixtures
//! do, takes that empty answer as the platform's word and leaves every authored key unconfirmed.
//! So the spawn, the topology scan and the derivation have to land on one frame in that order, and
//! this test reads the reported set rather than merely the completion count in order to say so.
//!
//! A completed scan is the invariant. A display topology carrying an entry nothing can name is
//! *not* one, because a display publishing no durable identity is a case this crate models
//! deliberately — `DisplayTestDescriptor::without_published_identity` exists for it. The reporter
//! is where the consuming app's stall lives: a failed scan never advances
//! `ReporterHealth::completed_runs`, and that counter is the only thing a waiting role or a
//! dependent batch is watching.
//!
//! # What these tests do not reach
//!
//! Measured rather than assumed: both tests pass on the pre-repair tree as well as the repaired
//! one, so neither reproduces the stall. They document the wiring a consumer builds and they pin
//! two invariants — that the reporter keeps completing scans beside a consumer's monitor entities,
//! and which displays those scans report. That is what they are for; they are not the regression
//! test for the stall and must not be counted as one. For the scripted case the reason is plain:
//! the fresh current-display list and the observed topology are written from one script, so they
//! agree at every revision and nothing between them ever fires. For the unscripted case the reason
//! is not established, and it is left recorded as an open question.
//!
//! Two parts of the regression sit outside their reach altogether. The frame ordering described
//! above is stated here but not enforced by an assertion. And the capture-address join is
//! invisible from inside this crate: a scripted display's [`PlatformDeviceHandle`] is the address
//! a capture attaches to, not identity material, and it never reaches the durable key — so the
//! reported set stays correct while that address is blank. Reaching it needs a consuming app
//! carrying rigging roles and a screen reporter to join against, which this crate has none of.
//!
//! The adapter repair itself is pinned by `hana`'s own tests under `crates/hana/src/screens`,
//! where a consuming app with rigging roles and a screen reporter can observe it — not by this
//! module.

use bevy::MinimalPlugins;
use bevy::prelude::App;
use bevy::prelude::IVec2;
use bevy::prelude::PreStartup;
use bevy::prelude::default;
use bevy::window::ExitCondition;
use bevy::window::Monitor;
use bevy::window::WindowPlugin;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::Devices;
use hana_rigging::prelude::PlatformDeviceHandle;
use hana_rigging::prelude::ReporterHealth;
use hana_rigging::prelude::ReporterId;
use tempfile::TempDir;
use tempfile::tempdir;

use crate::DisplayTestAdapter;
use crate::DisplayTestDescriptor;
use crate::DisplayTestDeviceKey;
use crate::DisplayTestReporterLookup;
use crate::WindowManagerPlugin;
use crate::visibility::tests;

/// The one display the scripted platform attaches in these tests.
const SCRIPTED_DISPLAY: &str = "scripted topology display";

/// Frames the app is driven for before the reporter is read.
///
/// A scan reaches its outcome across a handful of frames: the adapter publishes its monitor
/// entities in `Update`, the production topology scan reads them on the frame after, and the
/// reporter's discovery is queued and then run. A small ceiling is what keeps a stalled reporter a
/// fast failure rather than a hang — a reporter that has not completed a scan within this many
/// frames is not going to complete one.
const SETTLING_FRAMES: usize = 16;

/// What the production display reporter has done once the app has settled.
///
/// The two readings answer different halves of the regression and neither alone is enough. A
/// reporter that completed no scan is the stall itself; a reporter that completed one but reported
/// a display the platform never attached would be a repair that traded a stall for a phantom.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DisplayReporterReading {
    completed_scans:   u64,
    reported_displays: Vec<DeviceKey>,
}

/// An app wired the way a consumer of this crate wires it, holding its temporary state directory.
///
/// The directory is carried alongside the app because `WindowManagerPlugin` writes window state
/// into it: dropping it while the app still runs would delete the path out from under the plugin.
struct ScriptedTopologyApp {
    app:           App,
    adapter:       DisplayTestAdapter,
    _window_state: TempDir,
}

impl ScriptedTopologyApp {
    /// Build the plugin set a consumer builds: the adapter first, then the production window
    /// manager, with no winit and no primary window.
    fn new(adapter: DisplayTestAdapter) -> Result<Self, String> {
        let window_state = tempdir()
            .map_err(|error| format!("failed to create the window state directory: {error}"))?;
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            },
            adapter.clone(),
            WindowManagerPlugin::with_path(window_state.path().join("windows.ron")),
        ));
        app.world_mut().run_schedule(PreStartup);
        Ok(Self {
            app,
            adapter,
            _window_state: window_state,
        })
    }

    /// Spawn the [`Monitor`] entity a consuming fixture spawns for its own display.
    ///
    /// This is the shape both of `hana`'s screens fixtures use: the app already carries the
    /// scripted platform, and it adds a monitor entity of its own so its panels and destinations
    /// have one to sit on. It is deliberately spawned after `PreStartup`, which is where a consumer
    /// spawns it — the production monitor scan has already taken its first reading by then.
    fn spawn_consumer_monitor(&mut self) {
        self.app.world_mut().spawn(Monitor {
            name:                    Some(String::from("consumer fixture display")),
            physical_height:         1_080,
            physical_width:          1_920,
            physical_position:       IVec2::ZERO,
            refresh_rate_millihertz: None,
            scale_factor:            1.0,
            video_modes:             Vec::new(),
        });
    }

    /// The kernel handle `WindowManagerPlugin` registered the production display reporter under.
    fn display_reporter(&self) -> Result<ReporterId, String> {
        match self.adapter.display_reporter(self.app.world()) {
            DisplayTestReporterLookup::Registered(reporter) => Ok(reporter),
            DisplayTestReporterLookup::NotRegistered => Err(String::from(
                "the window manager registered no production display reporter",
            )),
        }
    }

    /// Run the app a bounded number of frames, then read what the display reporter has done.
    ///
    /// Called twice by every test: once with the scripted platform alone, to take the baseline the
    /// startup scan establishes, and once after the consumer's monitor has joined the world.
    fn settled_reporter(&mut self) -> Result<DisplayReporterReading, String> {
        let reporter = self.display_reporter()?;
        for _ in 0..SETTLING_FRAMES {
            tests::advance(&mut self.app, 0.0);
        }
        let completed_scans = self
            .app
            .world()
            .iter_entities()
            .filter_map(|entity| entity.get::<ReporterHealth>())
            .find(|health| health.belongs_to(reporter))
            .map_or(0, ReporterHealth::completed_runs);
        let reported_displays = self
            .app
            .world()
            .get_resource::<Devices>()
            .ok_or_else(|| String::from("the app installed no kernel device registry"))?
            .states()
            .map(|state| state.key.clone())
            .collect();
        Ok(DisplayReporterReading {
            completed_scans,
            reported_displays,
        })
    }
}

/// A platform that attaches one display publishing durable identity material.
fn adapter_with_one_scripted_display() -> Result<(DisplayTestAdapter, DeviceKey), String> {
    let adapter = DisplayTestAdapter::new();
    adapter.observe_displays(vec![DisplayTestDescriptor::new(
        SCRIPTED_DISPLAY,
        b"scripted-topology-panel".to_vec(),
        PlatformDeviceHandle::PlatformHasNoConcept,
    )]);
    let DisplayTestDeviceKey::Keyed(device) = adapter.display_device_key(SCRIPTED_DISPLAY) else {
        return Err(String::from(
            "the adapter named no durable key for the display it was scripted with",
        ));
    };
    Ok((adapter, device))
}

/// Assert the reporter completed another scan after the consumer's monitor joined the world.
///
/// Comparing against `before` rather than against zero is the whole point. The reporter completes
/// a scan at startup, before any consumer entity exists, so a bare `completed_scans > 0` is
/// satisfied by that one scan and holds even when every subsequent scan fails — which is exactly
/// the regression. Only the advance past the baseline says the reporter is still answering.
///
/// This is kept apart from what the scan reported because a scan that never completes leaves the
/// reported set unchanged too, so an assertion on the reported set alone would pass on the very
/// stall it exists to catch.
fn assert_reporter_kept_completing_scans(
    before: &DisplayReporterReading,
    after: &DisplayReporterReading,
    what_was_wired: &str,
) {
    assert!(
        after.completed_scans > before.completed_scans,
        "with {what_was_wired}, the production display reporter must complete another scan once \
         the consumer's monitor entity has joined the observed topology; the reporter is \
         registered as required, so a reporter that stops advancing its completed runs leaves \
         every waiting role and every dependent batch to time out — it stood at {before:?} with \
         the platform alone and at {after:?} afterwards"
    );
}

#[test]
fn an_unscripted_platform_completes_a_scan_reporting_no_display() -> Result<(), String> {
    // `hana`'s screen presentation fixture wires the adapter without ever scripting a display: it
    // runs the production window machinery, and drives a display reporter of its own. A platform
    // that attached nothing must still answer — with a completed scan naming no display — however
    // many monitor entities the consumer put in the world beside it.
    let mut fixture = ScriptedTopologyApp::new(DisplayTestAdapter::new())?;
    let with_platform_alone = fixture.settled_reporter()?;

    fixture.spawn_consumer_monitor();
    let reading = fixture.settled_reporter()?;

    assert_reporter_kept_completing_scans(
        &with_platform_alone,
        &reading,
        "an unscripted platform and a monitor entity the consumer spawned",
    );
    assert_eq!(
        reading.reported_displays,
        Vec::new(),
        "the scripted platform attached no display, so the reporter must name none; a monitor \
         entity the consumer spawned is ordinary Bevy topology, not attached hardware"
    );
    Ok(())
}

#[test]
fn a_scripted_platform_reports_only_its_own_display_beside_a_consumer_monitor() -> Result<(), String>
{
    let (adapter, scripted_device) = adapter_with_one_scripted_display()?;
    let mut fixture = ScriptedTopologyApp::new(adapter)?;
    let with_platform_alone = fixture.settled_reporter()?;

    fixture.spawn_consumer_monitor();
    let reading = fixture.settled_reporter()?;

    assert_reporter_kept_completing_scans(
        &with_platform_alone,
        &reading,
        "one scripted display beside a monitor entity the consumer spawned",
    );
    assert_eq!(
        reading.reported_displays,
        vec![scripted_device],
        "the reporter must name exactly the display the platform attached, under the durable key \
         the adapter names for it — an extra entry is the consumer's own monitor reported as \
         hardware, and a missing one is the attached display going unreported"
    );
    Ok(())
}
