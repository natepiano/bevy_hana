//! The shipped window driver, put through `hana_rigging_scripted`'s conformance walk.
//!
//! Every other test in this crate drives `WindowEndpointDriver` through a harness written for
//! window placement alone, so the states it proves are the states a window author thought to write
//! down. The walk proves the other half: that this driver reaches every state the kernel promises
//! any endpoint driver, refuses none of them, and keeps the two promises no kernel reading can
//! see — a session lease actually filed, and every cleanup verb actually acted on.
//!
//! The module lives in the crate rather than under `crates/hana_clerestory/tests/`. That directory
//! is `#![cfg(feature = "test")]` and the local gate never enables that feature, so a file placed
//! there runs no test at all. [`DisplayTestAdapter`] and
//! [`InjectedWinitWindows`](crate::restore::InjectedWinitWindows) are also `cfg(test)`-shaped, and
//! an in-crate `#[cfg(test)]` module is the only place they and the shipped driver exist at once.
//! `placement_retry_tests` is the crate's precedent for that placement.
//!
//! # What the subject has to supply that a hand-written fake would not
//!
//! `WindowEndpointDriver::resolve_target` waits on two things the walk never sets up, and
//! both have to be standing before the walk's first attempt can be issued.
//!
//! The first is a window attached to the role entity. `window_for_role_entity` reads the
//! `WindowRiggingRole` relationship, and with nothing attached the driver answers
//! `TargetWait::ApplicationRoleAttachmentRequired` forever, which is the deferral an application
//! that has not yet spawned its window produces. [`ConformanceSubject::attach_role`] is where the
//! application half of that contract is played, and it is idempotent per entity because the walk
//! calls it again on the successor of its replacement step, which reuses the displaced role entity.
//!
//! The second is the capability. `resolve_target` asks
//! [`required_capability::<LiveDisplayEndpoint>`](hana_rigging::TargetResolutionContext::required_capability)
//! for the monitor entity behind the bound display, and that lookup matches on the pair of reporter
//! and requirement — so only a declaration published under the reporter the driver reads can
//! satisfy it. The walk owns the only reporter it will publish a declaration through, so
//! [`ConformanceSubject::declare`] names the endpoint and `install` points [`MonitorReporterId`] at
//! the walk's reporter. `resolve_target` reads that resource at call time rather than from a value
//! captured at plugin build, which is what makes the override reach it at all.
//!
//! The monitor entity the declaration names does not exist when `declare` runs: the walk's `open`
//! calls `declare` before it builds the app and before `install`, and
//! [`CapabilityDeclaration::rebuilt_by`] takes a closure with no world access that the scripted
//! reporter re-runs on every replay of the scan. The closure therefore reads the entity out of a
//! shared [`OnceLock`] the subject holds, and `install` fills it by spawning the walk's one
//! [`Monitor`] before the first frame, which is when the first replay runs.
//!
//! # Two things the shipped plugin brings that the walk did not ask for
//!
//! `ConfiguredWindowManagerPlugin::build` adds `MonitorPlugin` unconditionally, and `MonitorPlugin`
//! registers `MonitorReporter` under `ReporterRegistration::required` with
//! `EstablishesAbsence(AllKeysOfKind { kind: DeviceKind::Display })`, which `DiscoveryControl`
//! cannot disable. It does no harm. It reports the adapter's scripted displays under the
//! `edid-serial` scheme, none of which is the walk's key, and the kernel answers `Present` for a
//! key any reporter reports present before it consults a covering reporter's omission — so the
//! walk's departures and returns come from the walk's own reporter. The adapter's records never
//! join the walk's key, which is the same decoupling the screen and camera subjects record.
//!
//! [`DisplayTestAdapter`] is added with an empty script, and it is added for what its plugin
//! installs rather than for anything it reports: `WinitMonitors`, `InjectedMonitorEvidence`, and
//! `InjectedWinitMonitorOrder`. `MinimalPlugins` carries no `WinitPlugin`, so without the adapter
//! the production monitor scan has no `WinitMonitors` to read and `init_monitors` cannot run at
//! all. An empty script also means the adapter publishes and despawns no `Monitor` of its own, so
//! the one the subject spawns is the whole topology the walk sees.
//!
//! # Why the subject widens the virtual clock
//!
//! This is the one place the subject departs from a plain plugin installation, and it is forced by
//! the restore pipeline rather than chosen.
//!
//! A window attempt does not end when the geometry is written. `check_restore_settling` holds the
//! attempt open until the observed window has been unchanged for `SETTLE_STABILITY_SECS`, and only
//! then does it hand the driver a completion. The walk paces a `NotMonitored` subject at one
//! millisecond of virtual time per frame and gives each step a thirty-two frame ceiling, so two
//! hundred milliseconds of stability is roughly six times more virtual time than the establishment
//! step will ever spend. The walk would refuse at `SessionEstablished` having proven nothing about
//! this driver.
//!
//! Widening `Time<Virtual>` is what reconciles the two. `TimeUpdateStrategy` — which the walk owns
//! and this subject never touches — sets the real delta; `Time<Virtual>` clamps that raw delta to
//! its own maximum and scales the clamped value, in that order. So the maximum is set to
//! [`WALK_CLOCK_GRAIN`], below any real step the walk can choose, which makes the clamp always
//! bind, and the relative speed is the exact ratio that turns that grain into
//! [`WALK_VIRTUAL_FRAME`]. Each walk frame then carries a quarter second, whatever real step the
//! walk chose for itself. Setting the maximum to the quarter second instead would bound the real
//! step rather than the virtual frame, and the scale would then multiply the walk's whole
//! millisecond — putting a thousand virtual seconds on every frame and expiring every deadline the
//! walk means to exercise on its first frame. No clock is added and none is replaced. The
//! kernel's own bounds are the walk's 600-second limits, which a walk of a few hundred quarter
//! seconds comes nowhere near, so the coarser step changes which frame each deadline falls on and
//! nothing about which deadlines exist.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::PoisonError;
use std::time::Duration;

use bevy::app::App;
use bevy::prelude::Entity;
use bevy::prelude::IVec2;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::On;
use bevy::prelude::Res;
use bevy::prelude::Resource;
use bevy::prelude::UVec2;
use bevy::prelude::Update;
use bevy::prelude::Window;
use bevy::prelude::World;
use bevy::prelude::default;
use bevy::prelude::resource_changed;
use bevy::time::Time;
use bevy::time::Virtual;
use bevy::window::ExitCondition;
use bevy::window::Monitor;
use bevy::window::OnMonitor;
use bevy::window::WindowMode;
use bevy::window::WindowPlugin;
use hana_rigging::prelude::Capabilities;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::EndpointDriverRegistration;
use hana_rigging::prelude::FlowExpectation;
use hana_rigging::prelude::ReporterId;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::SessionLookup;
use hana_rigging_scripted::CapabilityDeclaration;
use hana_rigging_scripted::ConformanceSubject;
use hana_rigging_scripted::RecordedCleanups;
use hana_rigging_scripted::ScriptedDevice;
use hana_rigging_scripted::ScriptedScan;
use hana_rigging_scripted::reported_key;
use hana_rigging_scripted::run;
use hana_rigging_scripted::scan;
use tempfile::TempDir;
use tempfile::tempdir;

use crate::DisplayTestAdapter;
use crate::Platform;
use crate::WindowManagerPlugin;
use crate::driver;
use crate::driver::WindowDriverId;
use crate::events::WindowRestoreMismatch;
use crate::managed;
use crate::managed::WindowBindingAuthoring;
use crate::managed::WindowRiggingRole;
use crate::monitors::CurrentMonitor;
use crate::monitors::CurrentMonitorEntity;
use crate::monitors::DisplayIdentity;
use crate::monitors::LiveDisplayEndpoint;
use crate::monitors::MonitorDescriptor;
use crate::monitors::MonitorReporterId;
use crate::monitors::MonitorTopologyRevision;
use crate::monitors::Monitors;
use crate::persistence::EstablishedWindowPlacement;
use crate::persistence::EstablishedWindowPosition;
use crate::persistence::SavedWindowMode;
use crate::restore::InjectedWinitWindows;

/// Identity space the walk's scripted display is named in.
///
/// The walk requires a reported device rather than a synthesized one, so the window subject names
/// its display in a scheme of its own instead of borrowing Clerestory's `edid-serial` scheme, which
/// belongs to displays a real platform scan produced and which the shipped `MonitorReporter` is
/// still reporting under throughout this walk.
const WINDOW_SCHEME: &str = "conformance-window-display";

/// The durable name the walk's one display is reported under.
const WINDOW_DEVICE: &str = "window-conformance";

/// File the shipped window manager persists its placements to.
///
/// Written into a temporary directory the subject owns, so a walk never reads or writes the
/// operator's real window state.
const WINDOW_STATE_FILE: &str = "windows.ron";

/// Geometry the walk's one monitor reports.
///
/// A commonplace desktop panel at unit scale. Nothing asserts these numbers directly; they are
/// large enough that [`WALK_WINDOW_OFFSET`] and [`WALK_WINDOW_LOGICAL_SIZE`] name a placement
/// wholly inside the display, which is what keeps the restore from being constrained into a
/// mismatch for a reason that says nothing about the driver. [`WalkFixtureBreach::RestoreMismatch`]
/// is what holds that claim to account: shrink this and the walk fails instead of quietly proving
/// less.
const WALK_MONITOR_SIZE: UVec2 = UVec2::new(1_920, 1_080);

/// Scale factor the walk's one monitor reports.
const WALK_MONITOR_SCALE: f64 = 1.0;

/// Position in the current enumeration the walk's one monitor takes.
///
/// The walk's topology is one display, so this is always the first and only entry.
const WALK_MONITOR_INDEX: usize = 0;

/// Logical offset from the walk's display that every binding asks its window to be placed at.
const WALK_WINDOW_OFFSET: IVec2 = IVec2::new(100, 100);

/// Logical size every binding asks its window to be restored to.
const WALK_WINDOW_LOGICAL_SIZE: UVec2 = UVec2::new(800, 600);

/// Virtual time one walk frame carries once the subject has widened the clock.
///
/// Longer than `SETTLE_STABILITY_SECS` so a motionless window settles on the frame after the one
/// that first observed it, and far short of `SETTLE_TIMEOUT_SECS` so the settle resolves as a match
/// rather than as an expired deadline. An expired deadline resolves through `emit_settle_mismatch`,
/// which is a success the kernel establishes on just as a match is, so
/// [`WalkFixtureBreach::RestoreMismatch`] is what separates the two here. The module doc holds the
/// whole reason this is not the walk's own millisecond.
const WALK_VIRTUAL_FRAME: Duration = Duration::from_millis(250);

/// Raw step every walk frame's delta is clamped to before the virtual clock scales it.
///
/// `Time<Virtual>::advance_with_raw_delta` clamps the raw delta to `max_delta` and multiplies by
/// the relative speed afterwards, so `max_delta` bounds the walk's real step rather than the
/// virtual frame that step produces. Set below any real step the walk can choose, the clamp always
/// binds, which is what lets [`WALK_CLOCK_WIDENING`] name an exact virtual frame instead of a
/// multiple of whatever step the walk happens to be using.
const WALK_CLOCK_GRAIN: Duration = Duration::from_micros(1);

/// Relative speed the subject sets on the virtual clock.
///
/// Every frame's delta is clamped to [`WALK_CLOCK_GRAIN`] first, so this is exactly
/// `WALK_VIRTUAL_FRAME / WALK_CLOCK_GRAIN` and each walk frame carries [`WALK_VIRTUAL_FRAME`] of
/// virtual time whatever real step the walk chose for itself.
const WALK_CLOCK_WIDENING: f32 = 250_000.0;

/// The widening above is [`WALK_VIRTUAL_FRAME`] counted in [`WALK_CLOCK_GRAIN`] units, and a `f32`
/// literal is the only way to write it. Checked here so a change to either duration fails the
/// build rather than quietly handing every walk frame the wrong amount of virtual time.
const _: () = assert!(
    WALK_CLOCK_GRAIN.as_micros() == 1 && WALK_VIRTUAL_FRAME.as_micros() == 250_000,
    "WALK_CLOCK_WIDENING no longer equals WALK_VIRTUAL_FRAME divided by WALK_CLOCK_GRAIN",
);

/// A promise the walk's fixture makes that no walk step can observe on its own.
///
/// Both used to live only in a constant's doc comment, where nothing could falsify them. A restore
/// that settles away from its target ends through `emit_settle_mismatch`, which reports a success
/// the kernel establishes on exactly as a match does, so an unreachable [`WALK_WINDOW_OFFSET`] or a
/// shrunken [`WALK_MONITOR_SIZE`] would leave the walk green while proving something other than
/// what it says it proves. A revised display topology reaches the walk's own scripted reporter as
/// an out-of-band scan request, which the walk's release accounting does not expect.
enum WalkFixtureBreach {
    /// A restore settled away from the geometry the driver dispatched it to.
    ///
    /// Boxed because the event carries sixteen expected/actual fields and the other variant is two
    /// words wide.
    RestoreMismatch(Box<WindowRestoreMismatch>),
    /// The display topology was revised after `init_monitors` installed the walk's one monitor.
    MonitorTopologyRevised {
        /// Revision `init_monitors` installed the walk's monitor under.
        installed: MonitorTopologyRevision,
        /// Revision observed afterwards, which the walk has no legitimate way to reach.
        observed:  MonitorTopologyRevision,
    },
}

impl Display for WalkFixtureBreach {
    /// Written out by hand rather than derived, because a derived `Debug` is not a read: every
    /// field here exists to name what broke in the failure message, and nothing else consumes them.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::RestoreMismatch(mismatch) => write!(
                formatter,
                "restore for role {} settled away from its target: {mismatch:#?}",
                mismatch.role,
            ),
            Self::MonitorTopologyRevised {
                installed,
                observed,
            } => write!(
                formatter,
                "the display topology was revised from {} to {} after `install`, so a dirty \
                 notification can reach the walk's own scripted reporter",
                installed.get(),
                observed.get(),
            ),
        }
    }
}

/// Fixture breaches recorded during the walk, shared between the subject and the test that owns it.
///
/// [`run`] takes the subject by value, so the test clones this handle before handing the subject
/// over and reads it once the walk has returned. Recorded rather than panicked on inside the
/// observer, so a breach is reported as a plain assertion naming the geometry instead of as a panic
/// unwinding out of a Bevy schedule.
#[derive(Clone, Default, Resource)]
struct WalkFixtureBreaches(Arc<Mutex<Vec<WalkFixtureBreach>>>);

impl WalkFixtureBreaches {
    fn record(&self, breach: WalkFixtureBreach) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(breach);
    }

    /// Every breach recorded so far, rendered for a failure message.
    fn recorded(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(ToString::to_string)
            .collect()
    }
}

/// Record any display-topology revision beyond the one `init_monitors` installed.
///
/// The walk spawns one [`Monitor`] in `install` and never touches the topology again, and the walk
/// depends on that rather than merely happening to enjoy it. A revision advance runs
/// `mark_monitor_reporter_dirty`,
/// which calls `DiscoveryControl::mark_dirty` on whatever [`MonitorReporterId`] names — and
/// `install` points that at the walk's scripted reporter, whose releases the walk's own run gate
/// and release ceiling account for. Today the only thing keeping that out-of-band scan request off
/// the walk's reporter is `MonitorDiscoveryRequestCoverage` covering the initial revision, while
/// `update_monitors` goes on re-running for the whole walk. This turns that coincidence into an
/// enforced invariant: a change that lets a dirty notification reach the scripted reporter fails
/// the walk instead of being absorbed by it.
fn record_topology_revisions(
    revision: Res<MonitorTopologyRevision>,
    breaches: Res<WalkFixtureBreaches>,
) {
    // `init_monitors` installs `MonitorTopologyRevision::default()`, so the baseline is a constant
    // rather than something to capture. Reading it from the resource's own first observed value
    // would depend on this system running before whatever advanced it, which nothing orders.
    let installed = MonitorTopologyRevision::default();
    if installed != *revision {
        breaches.record(WalkFixtureBreach::MonitorTopologyRevised {
            installed,
            observed: *revision,
        });
    }
}

/// Record a restore the driver settled away from the geometry it dispatched.
fn record_restore_mismatch(
    mismatch: On<WindowRestoreMismatch>,
    breaches: Res<WalkFixtureBreaches>,
) {
    breaches.record(WalkFixtureBreach::RestoreMismatch(Box::new(
        WindowRestoreMismatch::clone(&mismatch),
    )));
}

/// The shipped window driver as a conformance subject.
///
/// Holds no driver state: `WindowEndpointDriver` is registered by
/// `ConfiguredWindowManagerPlugin::build` and `add_endpoint_driver` consumes it, so the subject's
/// two readers go through the driver module's free `#[cfg(test)]` functions
/// `window_attempt_lookup` and `window_session_lookup`, which read the ledger out of the world,
/// instead of through a value held here.
struct WindowPlacementSubject {
    /// Supplies the resources the production monitor scan reads in place of winit's.
    adapter:      DisplayTestAdapter,
    /// Owns the directory the shipped window manager persists placements into.
    ///
    /// Held for the life of the subject because the plugin keeps the path and writes to it
    /// throughout the walk; dropping it would delete the directory under a live plugin.
    window_state: TempDir,
    /// The one monitor entity, shared with the declaration closure the reporter replays.
    ///
    /// A [`OnceLock`] rather than a field because `declare` runs before the app exists and
    /// `install` is what can spawn the entity. The closure the reporter keeps reads whatever is in
    /// here at replay time, which is always after `install` filled it.
    monitor:      Arc<OnceLock<Entity>>,
    /// The fixture promises the walk cannot see from its own steps, shared with the test.
    breaches:     WalkFixtureBreaches,
}

impl WindowPlacementSubject {
    fn new() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            adapter:      DisplayTestAdapter::new(),
            window_state: tempdir()?,
            monitor:      Arc::new(OnceLock::new()),
            breaches:     WalkFixtureBreaches::default(),
        })
    }

    /// The handle the test keeps before [`run`] takes the subject by value.
    fn breaches(&self) -> WalkFixtureBreaches { self.breaches.clone() }

    /// The geometry the walk's monitor entity reports, as the production scan will read it back.
    ///
    /// Built here rather than restated at each use so the [`Monitor`] the subject spawns and the
    /// [`MonitorDescriptor`] the declaration publishes cannot drift.
    const fn monitor_descriptor() -> MonitorDescriptor {
        MonitorDescriptor::for_current_enumeration(
            WALK_MONITOR_INDEX,
            WALK_MONITOR_SCALE,
            IVec2::ZERO,
            WALK_MONITOR_SIZE,
        )
    }

    /// The Bevy monitor the subject spawns as the walk's whole display topology.
    fn monitor() -> Monitor {
        let descriptor = Self::monitor_descriptor();
        Monitor {
            name:                    None,
            physical_height:         descriptor.physical_size.y,
            physical_width:          descriptor.physical_size.x,
            physical_position:       descriptor.physical_position,
            refresh_rate_millihertz: None,
            scale_factor:            descriptor.scale,
            video_modes:             Vec::new(),
        }
    }
}

impl ConformanceSubject for WindowPlacementSubject {
    type Configuration = EstablishedWindowPlacement;

    /// A window binding carries no stream, so nothing about its session is judged by data arrival.
    ///
    /// The driver no longer names the evidence at all: `DriverLedger` holds a
    /// `SessionDatumArrivalEvidence` slot per role, starts it at `NoDatumObserved`, folds every
    /// classified observation into it, and spends whatever it holds when the lease is taken. This
    /// driver classifies nothing — a placed window goes on being placed without producing anything
    /// further — so the evidence the ledger supplies at establishment is the silence it started
    /// from. The walk therefore asserts `Connected(FlowNotMonitored)` and never waits for a datum
    /// this driver would never credit.
    const FLOW: FlowExpectation = FlowExpectation::NotMonitored;

    #[expect(
        clippy::expect_used,
        reason = "`install` runs before the walk's first frame and has no way to report a failure \
                  through its return type; a window manager plugin that did not register its own \
                  driver is a broken fixture, and stopping here names it rather than letting the \
                  walk deadlock at target resolution"
    )]
    fn install(
        &mut self,
        app: &mut App,
        reporter: ReporterId,
    ) -> EndpointDriverRegistration<Self::Configuration> {
        // Pin the platform before the plugin builds, so the walk asserts the same driver branches
        // on every host. `Platform::detect` reads the environment: on a headless Linux runner it
        // reports `X11`, whose windowed restore waits forever for the `_NET_FRAME_EXTENTS` reply
        // that gates `X11FrameCompensated`, and the walk then refuses at `SessionEstablished`
        // having proven nothing about this driver. The plugin harness in `lib.rs` pins the same
        // way.
        app.insert_resource(Platform::MacOs);

        // The adapter goes in before the window manager, because `MonitorPlugin::build` reads
        // `DisplayTestAdapter::installation` out of the world to decide between the production
        // display source and the scripted one.
        app.add_plugins((
            WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            },
            self.adapter.clone(),
            WindowManagerPlugin::with_path(self.window_state.path().join(WINDOW_STATE_FILE)),
        ));

        // Read back rather than registered here: `ConfiguredWindowManagerPlugin::build` calls
        // `add_endpoint_driver` itself, and a second registration would give the walk a driver no
        // Clerestory system holds the state for.
        let driver = app
            .world()
            .get_resource::<WindowDriverId>()
            .expect("the window manager plugin registers its own endpoint driver")
            .0;

        // Spawned before the first frame, which is when the scripted reporter first replays the
        // declaration that reads this entity back out of the lock.
        let monitor = app.world_mut().spawn(Self::monitor()).id();
        let _ = self.monitor.set(monitor);

        // After the plugins, so this replaces the reporter `MonitorPlugin` registered rather than
        // being replaced by it. `resolve_target` reads this resource at call time, so the override
        // is what makes the walk's declaration satisfy the driver's requirement — which is the
        // whole point of the walk, since the driver has to resolve under the reporter `install`
        // receives.
        //
        // Two readers come along with it. `DisplayTestAdapter::display_reporter` only answers a
        // question, and nothing in this walk asks it. `mark_monitor_reporter_dirty` is the one that
        // matters: it sends `DiscoveryControl::mark_dirty` to whatever this names, so after the
        // override an out-of-band scan request would land on the walk's own scripted reporter. It
        // fires on a `MonitorTopologyRevision` advance, and `record_topology_revisions` below is
        // what makes "the topology never advances after `install`" an invariant the walk enforces
        // rather than a coincidence it relies on.
        app.insert_resource(MonitorReporterId::new(reporter));

        // Both fixture promises the walk asserts but no walk step can observe. The resource is a
        // clone of the handle the test kept, so what these record survives the subject.
        app.insert_resource(self.breaches.clone())
            .add_observer(record_restore_mismatch)
            .add_systems(
                Update,
                record_topology_revisions.run_if(resource_changed::<MonitorTopologyRevision>),
            );

        // The restore pipeline will not prepare a target for a window with no native window behind
        // it, and this walk has no winit. The resource is empty here; `attach_role` records each
        // window as it authors it.
        app.world_mut().init_resource::<InjectedWinitWindows>();

        let mut virtual_time = app.world_mut().resource_mut::<Time<Virtual>>();
        virtual_time.set_max_delta(WALK_CLOCK_GRAIN);
        virtual_time.set_relative_speed(WALK_CLOCK_WIDENING);

        driver
    }

    fn requested(&self) -> Self::Configuration {
        EstablishedWindowPlacement {
            position:          EstablishedWindowPosition::Restorable {
                logical_offset: WALK_WINDOW_OFFSET,
            },
            logical_size:      WALK_WINDOW_LOGICAL_SIZE,
            saved_window_mode: SavedWindowMode::Windowed,
        }
    }

    fn declare(&mut self, _: &DeviceKey) -> CapabilityDeclaration {
        // Named by the monitor entity rather than by the device, because that is what
        // `resolve_target` takes out of the endpoint: the device key reaches the driver through the
        // binding, and the monitor is the one fact only the display topology can supply.
        let monitor = Arc::clone(&self.monitor);
        CapabilityDeclaration::rebuilt_by(move || {
            Capabilities::new().with(LiveDisplayEndpoint {
                monitor:         monitor.get().copied().unwrap_or(Entity::PLACEHOLDER),
                descriptor:      Self::monitor_descriptor(),
                // The walk's display is named by the walk's own scheme, and the legacy identity is
                // read only when migrating a pre-v5 saved file. Anonymous is what a display with
                // no migration history carries.
                legacy_identity: DisplayIdentity::Anonymous,
            })
        })
    }

    #[expect(
        clippy::expect_used,
        reason = "the walk's monitor is spawned in `install` and its topology never changes, so a \
                  missing entry means the production monitor scan stopped installing the entity \
                  this subject spawned — a broken fixture the walk cannot proceed past, and one \
                  worth naming here rather than as an unresolvable target deferral"
    )]
    fn attach_role(&mut self, world: &mut World, role_entity: Entity) {
        // Idempotent per entity: `replace_binding` reuses the displaced role entity, so the walk's
        // replacement step calls this a second time on a role that already has its window. A second
        // window would give `window_for_role_entity` two answers and the driver would place
        // whichever the relationship happened to list first.
        if managed::window_for_role_entity(world, role_entity).is_some() {
            return;
        }
        let monitor = *self
            .monitor
            .get()
            .expect("`install` fills the monitor lock before the walk's first frame");
        let descriptor = world
            .resource::<Monitors>()
            .iter()
            .find(|live| live.entity == monitor)
            .map(|live| *live.descriptor)
            .expect("the walk's monitor entity is installed in the production topology");
        // Read from the installed topology rather than restated, because
        // `exact_monitor_association` compares this descriptor against the installed one by value
        // and the restore pipeline skips any window whose association is not exact.
        let window = world
            .spawn((
                Window {
                    visible: false,
                    ..default()
                },
                OnMonitor(monitor),
                CurrentMonitor {
                    descriptor,
                    effective_window_mode: WindowMode::Windowed,
                },
                CurrentMonitorEntity::new(monitor),
                // Clerestory authors a binding of its own for a managed window. This window is
                // already bound — by the walk — so it carries the authoring verdict a registered
                // window carries and no `ManagedWindow` marker to draw the authoring path to it.
                WindowBindingAuthoring::Registered,
                WindowRiggingRole::new(role_entity),
            ))
            .id();
        world
            .resource_mut::<InjectedWinitWindows>()
            .insert(window, UVec2::ZERO);
    }

    fn session(&self, world: &World, role: &RoleKey) -> SessionLookup {
        driver::window_session_lookup(world, role)
    }

    fn cleanups(&self, world: &World, role: &RoleKey) -> RecordedCleanups {
        driver::recorded_window_cleanups(world, role)
    }
}

/// One scripted scan reporting the walk's display as present.
///
/// The walk builds its own departure and return scans around this one, so the subject supplies the
/// present set alone.
fn window_scan() -> Result<ScriptedScan, Box<dyn Error>> {
    Ok(scan![ScriptedDevice::present(reported_key(
        DeviceKind::Display,
        WINDOW_SCHEME,
        WINDOW_DEVICE,
    )?)])
}

/// The shipped window driver walks the kernel's whole endpoint lifecycle.
///
/// Green here means more than "window placement still works": it means this driver files the lease
/// the kernel issues, unwinds every attempt the kernel cancels, releases every session the kernel
/// ends — under a departure, a retirement, a replacement, and a role entity already despawned — and
/// holds a placed window connected for as long as the kernel says the session lives. The readings
/// behind the last of those are the driver's own record, which no kernel observable can reach.
#[test]
fn the_shipped_window_driver_walks_the_whole_kernel_lifecycle() -> Result<(), Box<dyn Error>> {
    let subject = WindowPlacementSubject::new()?;
    let breaches = subject.breaches();
    // The walk returns `Ok` only on a whole pass, so the returned report has already reached every
    // step; re-asserting any of them here would assert something that cannot be false.
    let walked = run(subject, window_scan()?);
    // Read before the walk's own result is propagated. A breach is a fixture that stopped meaning
    // what its doc says, and it explains a step failure that would otherwise read as a driver
    // defect; when the walk passed, it is the only thing that can still be wrong.
    let recorded = breaches.recorded();
    assert!(
        recorded.is_empty(),
        "the walk's fixture promises were breached:\n{}",
        recorded.join("\n"),
    );
    walked?;
    Ok(())
}
