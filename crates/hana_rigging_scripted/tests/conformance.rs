//! Controls for the conformance suite: the fake that keeps the driver contract, and the fakes that
//! break exactly one promise each.
//!
//! A conformance suite is only worth the frames it spends if it can go red, and these cases are how
//! that is established. One fake keeps every promise the contract makes and must pass; the others
//! are that same fake with one clause removed, and each must fail — so a green run of the suite
//! against a real driver means the driver kept its promises, not that the walk cannot tell.
//!
//! Every fake here is built on [`DriverLedger`], because that is the shape the specification says a
//! driver takes: hardware records beside one ledger verb per callback. A fake that hand-rolled its
//! own bookkeeping would prove the walk against a driver no shipped kernel resembles.
//!
//! The fakes finish their own attempts. `start_apply` begins the attempt and succeeds it in the
//! same call, which is what a driver whose device answers synchronously does, and it is what lets
//! the walk drive itself: the suite hands out no completion hook, so a fake that waited for one
//! would stall every case in this file rather than the one case each is about. The kernel drains
//! queued completions at the top of an update and dispatches `start_apply` later in the same one,
//! so a completion filed inside `start_apply` is not consumed until the next update and the attempt
//! stays observably in flight for a frame — which is what keeps the walk's two mid-flight steps
//! reachable for a synchronous subject.
//!
//! [`FlowExpectation`] is an associated const on the subject, so a monitored subject and an
//! unmonitored one are two types rather than one type with a field. The streaming pair exists for
//! that reason: a stall control that only ever asserts failure would be satisfied just as well by a
//! suite that failed every monitored subject on principle, so the crediting streamer is what makes
//! the silent one mean anything.
//!
//! Every fake reaches the app through a plugin of its own, which is what `install` documents and
//! what both shipped drivers do: the plugin registers the driver and publishes its route, and
//! `install` hands back the route the plugin produced. A fake that called `add_endpoint_driver`
//! inline would compile and pass while leaving that half of the contract proven by nothing.
//!
//! The fakes also record every cleanup verb the kernel dispatches to them. That record, not any
//! kernel reading, is what the walk's teardown steps are measured against: the kernel publishes an
//! identical role status whether a driver unwound its attempt or ignored the call, so a driver with
//! an empty `cancel_apply` is invisible to anything that reads only the kernel. One control here is
//! exactly that driver.
//!
//! One control resolves its target through neither shortcut: it waits for a capability the suite
//! declared for the presented device and for an entity `attach_role` puts on the role. Every other
//! fake answers `Reached` immediately, which would leave `declare` and `attach_role` — two of the
//! subject contract's six members — exercised by nothing at all.
//!
//! Crediting is where a real streaming driver does it — in a system of the driver's own, installed
//! by the driver's own `install`, never inside a kernel callback. That is why these subjects can
//! promise continuous flow and keep the promise: `install` is the hook, and the trait needs none.

use std::error::Error;
use std::hash::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::time::Duration;

use bevy::app::App;
use bevy::app::Plugin;
use bevy::app::Update;
use bevy::ecs::reflect::ReflectComponent;
use bevy::platform::time::Instant;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Reflect;
use bevy::prelude::Resource;
use bevy::prelude::World;
use bevy::time::Real;
use bevy::time::Time;
use hana_rigging::prelude::Applied;
use hana_rigging::prelude::ApplyContext;
use hana_rigging::prelude::AttemptInvalidation;
use hana_rigging::prelude::AttemptRef;
use hana_rigging::prelude::CancelledAttempt;
use hana_rigging::prelude::Capabilities;
use hana_rigging::prelude::ContinuousFlowExpectation;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::DeviceTransport;
use hana_rigging::prelude::DiscardedSession;
use hana_rigging::prelude::DriverCleanupRoleEntity;
use hana_rigging::prelude::DriverLedger;
use hana_rigging::prelude::EndpointDriver;
use hana_rigging::prelude::EndpointDriverRegistration;
use hana_rigging::prelude::EstablishedContext;
use hana_rigging::prelude::Establishing;
use hana_rigging::prelude::Establishment;
use hana_rigging::prelude::FirstDatumTimeout;
use hana_rigging::prelude::FlowExpectation;
use hana_rigging::prelude::MaximumDatumGap;
use hana_rigging::prelude::ReleasedLease;
use hana_rigging::prelude::ReporterId;
use hana_rigging::prelude::RiggingAppExt;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::SessionDatumArrivalEvidence;
use hana_rigging::prelude::SessionLookup;
use hana_rigging::prelude::SessionRef;
use hana_rigging::prelude::SessionReleaseCause;
use hana_rigging::prelude::TargetResolution;
use hana_rigging::prelude::TargetResolutionContext;
use hana_rigging::prelude::TargetWait;
use hana_rigging::prelude::TransportObservation;
use hana_rigging_scripted::CapabilityDeclaration;
use hana_rigging_scripted::ConformanceRefusal;
use hana_rigging_scripted::ConformanceStep;
use hana_rigging_scripted::ConformanceStop;
use hana_rigging_scripted::ConformanceSubject;
use hana_rigging_scripted::RecordedCleanup;
use hana_rigging_scripted::RecordedCleanups;
use hana_rigging_scripted::ScriptedDevice;
use hana_rigging_scripted::ScriptedScan;
use hana_rigging_scripted::reported_key;
use hana_rigging_scripted::run;
use hana_rigging_scripted::scan;

/// Identity space every scripted panel in this file is named in.
const PANEL_SCHEME: &str = "usb-serial";

/// Slot every scripted role asks its driver for.
///
/// The value is never read back: these controls are about the lifecycle, and a configuration that
/// varied would only give a case a second way to go red.
const REQUESTED_SLOT: u32 = 1;

/// How long a monitored session may wait for its first datum before the kernel calls it stalled.
///
/// This is the bound the silent streamer trips, so it is short enough to be crossed inside the
/// walk's frame budget rather than outliving it. The crediting streamer never meets it at all: it
/// credits before its lease exists, so the ledger retains that observation and its session opens
/// already past its first datum.
const FIRST_DATUM_BOUND: Duration = Duration::from_millis(250);

/// How long a flowing session may go between data before the kernel calls it stalled.
///
/// Deliberately far larger than the first-datum bound, and larger than any frame step the suite
/// could reasonably choose. The crediting streamer credits once per frame, so a gap bound near the
/// frame step would stall it on the suite's own pacing and the passing control would report on the
/// walk's clock rather than on the driver.
///
/// The declared gap in this file's coverage: no control here crosses this bound, so the stall the
/// silent streamer proves is always `FirstDatumOverdue` and never `MaximumDatumGapExceeded`. A
/// control for the second cause would have to declare a gap small enough for the walk's clock to
/// cross, and the walk derives its frame step from the tightest bound a subject declares — so such
/// a control would be measuring the suite's pacing against itself rather than measuring a driver.
/// The two causes travel the same code path from `judge_continuous_flow`, and the bound-crossing
/// arithmetic is the kernel's own tested ground, so the gap is narrow and deliberate.
const DATUM_GAP_BOUND: Duration = Duration::from_secs(60);

/// Placement the scripted role asks its driver for.
///
/// Deliberately not `Clone`: the contract puts no `Clone` bound on `Configuration`, and a
/// configuration that cannot be cloned is what makes the suite prove it never needed one.
#[derive(Component, Debug, PartialEq, Reflect)]
#[reflect(Component)]
struct PanelPlacement {
    slot: u32,
}

/// One observation of a scripted panel's transport: a datum reached its consumer.
///
/// A single shape is enough here. These fakes either deliver data or deliver none, and a fake with
/// a richer transport would be testing [`DeviceTransport::classify`] rather than the walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PanelDatum;

/// The scripted panel's transport, so a credit has a real classification to route.
struct PanelTransport;

impl DeviceTransport for PanelTransport {
    type Observation = PanelDatum;

    fn classify(_: &Self::Observation) -> TransportObservation {
        TransportObservation::DatumDelivered
    }
}

/// What the fake does with the lease the kernel issues at establishment.
///
/// The two arms are the whole difference between the passing control and the lease-less one, so the
/// defect under test sits one enum away from the driver that keeps the contract, rather than in a
/// second driver a reader has to diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeaseDiscipline {
    /// File the lease in the ledger, which is the establishment half of the contract.
    Filed,
    /// Take the lease and drop it unfiled.
    ///
    /// Nothing in `hana_rigging` notices at the moment it happens: a session lease has no drop
    /// behaviour, and the kernel issues the lease and publishes establishment before the driver is
    /// ever called, so every kernel reading stays healthy. The broken promise shows only when the
    /// subject is asked what its own record holds.
    Dropped,
}

/// What the fake does when the kernel cancels an attempt it had started.
///
/// The second arm is the defect the walk's two cancellation steps exist to catch, and it is written
/// as the absence of a body rather than as a wrong body: a driver that simply never wrote its
/// cancellation path is the shape a real one degrades into, and it is the shape a walk that reads
/// only kernel state cannot see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CancelDiscipline {
    /// Record the cancellation and drop the attempt's ledger entry, which is the whole contract.
    Unwinding,
    /// Do nothing at all, leaving the ledger's attempt slot abandoned and the verb unobserved.
    Ignoring,
}

/// Whether the fake keeps its session's flow alive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlowDiscipline {
    /// Credit one datum per frame against the role the walk authored.
    Crediting,
    /// Credit nothing, so a monitored session runs out its first-datum bound.
    Silent,
}

/// The reporter this subject publishes its capability declaration under.
///
/// A shipped driver captures the id at plugin build and reads every capability against it —
/// `ScreenDriver` holds exactly this — so the fake here does the same. The pre-install variant
/// exists because a subject value is constructed before the walk has a reporter to hand it, and an
/// absence that names why is worth more to a reader than an empty option.
#[derive(Clone, Copy, Debug)]
enum DeclaringReporter {
    /// `install` has not run yet, so no reporter has been captured.
    NotYetInstalled,
    /// The walk's reporter, captured at install.
    Installed(ReporterId),
}

/// The bus line this subject declared for the device the walk presented.
///
/// Recorded rather than assumed so `resolve_target` can check that the capability reaching it is
/// the one `declare` built for *this* device. A driver that accepted any line would pass just as
/// well against a declaration published for some other unit, which is the thing the capability
/// channel exists to prevent.
#[derive(Clone, Copy, Debug)]
enum DeclaredBusLine {
    /// `declare` has not run yet, so this subject has named no line.
    NotYetDeclared,
    /// The line derived from the presented device key.
    Declared(u8),
}

/// One cleanup verb the kernel dispatched, kept beside the role it named.
///
/// The role travels with the record because the walk asks about one role at a time and
/// [`RecordedCleanup`] does not carry it: a driver serving several roles that answered with all of
/// them would report another role's teardown as this one's evidence.
#[derive(Clone, Debug)]
struct RoleCleanup {
    role:    RoleKey,
    cleanup: RecordedCleanup,
}

/// The role the walk authored, as the driver came to know it.
///
/// The suite owns the role key and the driver learns it only when an attempt arrives, so the
/// pre-attempt state is a named variant rather than an absence a reader has to interpret.
#[derive(Clone, Debug)]
enum ConformanceRole {
    /// No attempt has reached this driver yet.
    Unauthored,
    /// The walk authored this role, and every ledger question is asked about it.
    Authored(RoleKey),
}

/// Everything one fake driver holds: the kernel's authorities, and its own record of the role.
///
/// The split is the one the ledger's own documentation names. `ledger` holds every authority
/// `hana_rigging` issued; `role` is the driver's own record — the stand-in for a real driver's
/// capture session or window placement — and it is what the crediting system reads, because the
/// ledger deliberately hands out no role listing.
struct FakePanelState {
    ledger:            DriverLedger<PanelPlacement>,
    lease_discipline:  LeaseDiscipline,
    cancel_discipline: CancelDiscipline,
    reporter:          DeclaringReporter,
    declared_line:     DeclaredBusLine,
    role:              ConformanceRole,
    /// Every cleanup verb the kernel dispatched, in dispatch order.
    ///
    /// A driver's own record that it was told is the only evidence the walk can hold for the two
    /// cleanup verbs: the kernel publishes the same readings whether the driver unwound or ignored
    /// them, so a step that reads only the kernel is satisfied by a driver with an empty body.
    cleanups:          Vec<RoleCleanup>,
}

impl FakePanelState {
    fn new(lease_discipline: LeaseDiscipline, cancel_discipline: CancelDiscipline) -> Self {
        Self {
            ledger: DriverLedger::new(),
            lease_discipline,
            cancel_discipline,
            reporter: DeclaringReporter::NotYetInstalled,
            declared_line: DeclaredBusLine::NotYetDeclared,
            role: ConformanceRole::Unauthored,
            cleanups: Vec::new(),
        }
    }
}

/// A driver built the way the contract's own worked example is: hardware records beside one ledger.
struct FakePanelDriver {
    state: Arc<Mutex<FakePanelState>>,
}

impl FakePanelDriver {
    /// Build the driver from the state its subject shares with it.
    ///
    /// A free-standing constructor rather than a literal because [`ScriptedPanelPlugin`] is handed
    /// only `&self` and has to build its driver from a `fn` item.
    const fn new(state: Arc<Mutex<FakePanelState>>) -> Self { Self { state } }
}

impl EndpointDriver for FakePanelDriver {
    type Configuration = PanelPlacement;
    type Target = ();

    fn resolve_target(
        &mut self,
        _: &mut World,
        _: &TargetResolutionContext<'_>,
        _: &Self::Configuration,
    ) -> TargetResolution<Self::Target> {
        TargetResolution::Reached(())
    }

    fn start_apply(
        &mut self,
        _: &mut World,
        context: ApplyContext<'_, Self::Configuration>,
        _: &Self::Configuration,
        (): Self::Target,
    ) {
        let role = context.target().role().clone();
        let mut state = lock(&self.state);
        state.role = ConformanceRole::Authored(role);
        let attempt = state.ledger.begin_attempt(context, ()).attempt();
        state.ledger.succeed_attempt(attempt, Applied::AsDispatched);
    }

    fn established(&mut self, _: &mut World, context: EstablishedContext<'_, Self::Configuration>) {
        let mut state = lock(&self.state);
        match state.lease_discipline {
            LeaseDiscipline::Filed => {
                // Every outcome asks the same of this driver, because the ledger is its only
                // record: an establishment stands, a displaced predecessor's record was replaced
                // when this attempt began, and a refusal was already reported on the lease. A
                // driver holding hardware would branch here; that branching is the shipped
                // drivers' business and not what the walk is measuring.
                let _: Establishment<(), ()> =
                    state
                        .ledger
                        .establish_lease(context, Establishing::Live, |()| ());
            },
            // The defect this control exists for: the lease is taken and dropped, so the ledger
            // files nothing and the driver holds no authority for a session the kernel records as
            // established.
            LeaseDiscipline::Dropped => {
                drop(context.into_lease(SessionDatumArrivalEvidence::NoDatumObserved));
            },
        }
        drop(state);
    }

    fn cancel_apply(
        &mut self,
        _: &mut World,
        role: &RoleKey,
        role_entity: DriverCleanupRoleEntity,
        attempt: AttemptRef,
        invalidation: AttemptInvalidation,
    ) {
        let mut state = lock(&self.state);
        match state.cancel_discipline {
            CancelDiscipline::Unwinding => {
                state.cleanups.push(RoleCleanup {
                    role:    role.clone(),
                    cleanup: RecordedCleanup::AttemptCancelled {
                        role_entity,
                        attempt,
                        invalidation,
                    },
                });
                // The answer tells a driver whether the work it started is worth unwinding. This
                // one started none beyond the ledger entry the verb just dropped.
                let _: CancelledAttempt<()> = state.ledger.cancel_attempt(role, attempt);
            },
            // The defect this control exists for: the verb arrives and the driver does nothing with
            // it, so the ledger keeps an attempt slot no attempt will ever finish and the driver
            // holds no record it was ever told.
            CancelDiscipline::Ignoring => {},
        }
        drop(state);
    }

    fn release_session(
        &mut self,
        _: &mut World,
        role: &RoleKey,
        role_entity: DriverCleanupRoleEntity,
        session: SessionRef,
        cause: SessionReleaseCause,
    ) {
        let mut state = lock(&self.state);
        state.cleanups.push(RoleCleanup {
            role:    role.clone(),
            cleanup: RecordedCleanup::SessionReleased {
                role_entity,
                session,
                cause,
            },
        });
        // The guard on hardware teardown: only `Released` names the session this role still held,
        // and tearing down on either other answer would end a successor's device work. This driver
        // holds no hardware beyond the ledger, so the only teardown it owes is the ledger's own —
        // and it owes that one. `release_lease` moves the record to the retained state and prunes
        // nothing, so a fake that stopped here would keep the role record alive through the grace
        // window: `credit_flow` would answer `RetainedForEstablishment` where the kernel expects
        // `UnknownRole`, the retained evidence would be spent into the next session, and the next
        // establishment would answer `EstablishedOverRetained` instead of `Established`. This is
        // what every shipped driver does immediately after `Released`.
        match state.ledger.release_lease(role, session) {
            ReleasedLease::Released => match state.ledger.discard_retained(role) {
                DiscardedSession::Discarded(()) | DiscardedSession::NothingRetained => {},
            },
            ReleasedLease::OtherSessionEstablished | ReleasedLease::NotEstablished => {},
        }
        drop(state);
    }
}

/// The driver state the per-frame crediting system reaches.
#[derive(Resource)]
struct CreditingPanel(Arc<Mutex<FakePanelState>>);

/// Credit one datum per frame against the role the walk authored.
///
/// This is where a real streaming driver credits: at the point a frame reached its consumer, from a
/// system of the driver's own, never from inside a kernel callback. Before a lease exists the
/// ledger retains the observation for establishment, so a session opens already flowing rather than
/// starting from a silence it never had.
fn credit_the_authored_role(world: &mut World) {
    let Some(driver) = world
        .get_resource::<CreditingPanel>()
        .map(|crediting| Arc::clone(&crediting.0))
    else {
        return;
    };
    let observed_at = frame_instant(world);
    let mut state = lock(&driver);
    let ConformanceRole::Authored(role) = state.role.clone() else {
        return;
    };
    state
        .ledger
        .credit_flow::<PanelTransport>(&role, &PanelDatum, observed_at);
}

/// A subject with no data axis, which is the walk stripped to the lifecycle alone.
struct UnmonitoredPanel {
    state: Arc<Mutex<FakePanelState>>,
}

impl UnmonitoredPanel {
    /// The control that keeps every promise the contract makes.
    fn keeping_the_contract() -> Self {
        Self::new(LeaseDiscipline::Filed, CancelDiscipline::Unwinding)
    }

    /// The control that keeps every promise but the lease.
    fn never_filing_its_lease() -> Self {
        Self::new(LeaseDiscipline::Dropped, CancelDiscipline::Unwinding)
    }

    /// The control whose `cancel_apply` has no body at all.
    fn never_unwinding_a_cancellation() -> Self {
        Self::new(LeaseDiscipline::Filed, CancelDiscipline::Ignoring)
    }

    fn new(lease_discipline: LeaseDiscipline, cancel_discipline: CancelDiscipline) -> Self {
        Self {
            state: Arc::new(Mutex::new(FakePanelState::new(
                lease_discipline,
                cancel_discipline,
            ))),
        }
    }
}

impl ConformanceSubject for UnmonitoredPanel {
    type Configuration = PanelPlacement;

    const FLOW: FlowExpectation = FlowExpectation::NotMonitored;

    fn install(
        &mut self,
        app: &mut App,
        reporter: ReporterId,
    ) -> EndpointDriverRegistration<Self::Configuration> {
        install_panel_plugin(
            app,
            &self.state,
            reporter,
            FlowDiscipline::Silent,
            FakePanelDriver::new,
        )
    }

    fn requested(&self) -> Self::Configuration {
        PanelPlacement {
            slot: REQUESTED_SLOT,
        }
    }

    fn declare(&mut self, _: &DeviceKey) -> CapabilityDeclaration { CapabilityDeclaration::none() }

    fn attach_role(&mut self, _: &mut World, _: Entity) {}

    fn session(&self, _: &World, role: &RoleKey) -> SessionLookup {
        lock(&self.state).ledger.session_of(role)
    }

    fn cleanups(&self, _: &World, role: &RoleKey) -> RecordedCleanups {
        recorded_cleanups(&self.state, role)
    }
}

/// A subject whose session must keep delivering data, which is the walk with the flow step live.
struct StreamingPanel {
    state: Arc<Mutex<FakePanelState>>,
    flow:  FlowDiscipline,
}

impl StreamingPanel {
    /// The control that keeps delivering, so the stall control below means something.
    fn delivering() -> Self { Self::new(FlowDiscipline::Crediting) }

    /// The control that establishes a session and then delivers nothing.
    fn silent() -> Self { Self::new(FlowDiscipline::Silent) }

    fn new(flow: FlowDiscipline) -> Self {
        Self {
            state: Arc::new(Mutex::new(FakePanelState::new(
                LeaseDiscipline::Filed,
                CancelDiscipline::Unwinding,
            ))),
            flow,
        }
    }
}

impl ConformanceSubject for StreamingPanel {
    type Configuration = PanelPlacement;

    const FLOW: FlowExpectation = FlowExpectation::Continuous(ContinuousFlowExpectation::new(
        first_datum_bound(),
        datum_gap_bound(),
    ));

    fn install(
        &mut self,
        app: &mut App,
        reporter: ReporterId,
    ) -> EndpointDriverRegistration<Self::Configuration> {
        install_panel_plugin(app, &self.state, reporter, self.flow, FakePanelDriver::new)
    }

    fn requested(&self) -> Self::Configuration {
        PanelPlacement {
            slot: REQUESTED_SLOT,
        }
    }

    fn declare(&mut self, _: &DeviceKey) -> CapabilityDeclaration { CapabilityDeclaration::none() }

    fn attach_role(&mut self, _: &mut World, _: Entity) {}

    fn session(&self, _: &World, role: &RoleKey) -> SessionLookup {
        lock(&self.state).ledger.session_of(role)
    }

    fn cleanups(&self, _: &World, role: &RoleKey) -> RecordedCleanups {
        recorded_cleanups(&self.state, role)
    }
}

/// The bus address a panel publishes, which its driver must read before it can drive anything.
///
/// This is the capability half of the subject contract: `declare` names it, the suite publishes it
/// on the presented device, and the driver resolves it through `required_capability` rather than
/// reading the device entity behind the kernel's back.
#[derive(Component, Debug, PartialEq, Reflect)]
#[reflect(Component)]
struct PanelBusAddress {
    line: u8,
}

/// The driven entity a window-attaching driver waits for, put on the role entity by `attach_role`.
///
/// A driver whose target is an application-owned entity cannot resolve until the application has
/// attached it, and `attach_role` is where the suite lets a subject do that. Without the marker the
/// driver below defers, which is what makes the attachment observable rather than assumed.
#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
struct AttachedPanelSurface;

/// A driver that reaches its target only once the capability and the attachment are both there.
///
/// Every other fake in this file resolves unconditionally, which leaves two clauses of the subject
/// contract — `declare` and `attach_role` — asserted by nothing. This one resolves through both.
struct AttachingPanelDriver {
    state: Arc<Mutex<FakePanelState>>,
}

impl AttachingPanelDriver {
    /// Build the driver from the state its subject shares with it, for the same reason as
    /// [`FakePanelDriver::new`].
    const fn new(state: Arc<Mutex<FakePanelState>>) -> Self { Self { state } }
}

impl EndpointDriver for AttachingPanelDriver {
    type Configuration = PanelPlacement;
    type Target = ();

    fn resolve_target(
        &mut self,
        world: &mut World,
        context: &TargetResolutionContext<'_>,
        _: &Self::Configuration,
    ) -> TargetResolution<Self::Target> {
        if world
            .get::<AttachedPanelSurface>(context.role_entity())
            .is_none()
        {
            return TargetResolution::Deferred(TargetWait::ApplicationRoleAttachmentRequired);
        }
        // The capability is resolved through the kernel rather than read off the device entity,
        // because `required_capability` is what a shipped driver calls: it answers only when the
        // reporter that published the declaration also projected it, so a capability present on the
        // entity but unprojected still defers. The reporter is the one captured at install.
        let (DeclaringReporter::Installed(reporter), DeclaredBusLine::Declared(declared)) = ({
            let state = lock(&self.state);
            (state.reporter, state.declared_line)
        }) else {
            return TargetResolution::Deferred(TargetWait::ApplicationRoleAttachmentRequired);
        };
        match context.required_capability::<PanelBusAddress>(world, reporter) {
            // The line is compared rather than accepted, so the capability that arrived is proven
            // to be the declaration this subject built for this device and not some other unit's.
            Ok(address) if address.line == declared => TargetResolution::Reached(()),
            // A line that is not the declared one belongs to some other unit, so the driver waits
            // rather than driving hardware this declaration never named.
            Ok(_) => TargetResolution::Deferred(TargetWait::ApplicationRoleAttachmentRequired),
            Err(unavailable) => TargetResolution::Deferred(unavailable.target_wait(world)),
        }
    }

    fn start_apply(
        &mut self,
        world: &mut World,
        context: ApplyContext<'_, Self::Configuration>,
        requested: &Self::Configuration,
        target: Self::Target,
    ) {
        FakePanelDriver::new(Arc::clone(&self.state))
            .start_apply(world, context, requested, target);
    }

    fn established(
        &mut self,
        world: &mut World,
        context: EstablishedContext<'_, Self::Configuration>,
    ) {
        FakePanelDriver::new(Arc::clone(&self.state)).established(world, context);
    }

    fn cancel_apply(
        &mut self,
        world: &mut World,
        role: &RoleKey,
        role_entity: DriverCleanupRoleEntity,
        attempt: AttemptRef,
        cause: AttemptInvalidation,
    ) {
        FakePanelDriver::new(Arc::clone(&self.state)).cancel_apply(
            world,
            role,
            role_entity,
            attempt,
            cause,
        );
    }

    fn release_session(
        &mut self,
        world: &mut World,
        role: &RoleKey,
        role_entity: DriverCleanupRoleEntity,
        session: SessionRef,
        cause: SessionReleaseCause,
    ) {
        FakePanelDriver::new(Arc::clone(&self.state)).release_session(
            world,
            role,
            role_entity,
            session,
            cause,
        );
    }
}

/// A subject whose target needs both a declared capability and an attached role entity.
struct AttachingPanel {
    state: Arc<Mutex<FakePanelState>>,
}

impl AttachingPanel {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(FakePanelState::new(
                LeaseDiscipline::Filed,
                CancelDiscipline::Unwinding,
            ))),
        }
    }
}

impl ConformanceSubject for AttachingPanel {
    type Configuration = PanelPlacement;

    const FLOW: FlowExpectation = FlowExpectation::NotMonitored;

    fn install(
        &mut self,
        app: &mut App,
        reporter: ReporterId,
    ) -> EndpointDriverRegistration<Self::Configuration> {
        install_panel_plugin(
            app,
            &self.state,
            reporter,
            FlowDiscipline::Silent,
            AttachingPanelDriver::new,
        )
    }

    fn requested(&self) -> Self::Configuration {
        PanelPlacement {
            slot: REQUESTED_SLOT,
        }
    }

    fn declare(&mut self, device: &DeviceKey) -> CapabilityDeclaration {
        // Derived from the presented device rather than fixed, because the only required capability
        // in the repo carries device-specific data and a declaration that could not name its device
        // would be a channel no shipped driver could use. The driver checks the line it gets back.
        let line = bus_line_for(device);
        lock(&self.state).declared_line = DeclaredBusLine::Declared(line);

        CapabilityDeclaration::rebuilt_by(move || {
            Capabilities::new().with(PanelBusAddress { line })
        })
    }

    fn attach_role(&mut self, world: &mut World, role_entity: Entity) {
        world.entity_mut(role_entity).insert(AttachedPanelSurface);
    }

    fn session(&self, _: &World, role: &RoleKey) -> SessionLookup {
        lock(&self.state).ledger.session_of(role)
    }

    fn cleanups(&self, _: &World, role: &RoleKey) -> RecordedCleanups {
        recorded_cleanups(&self.state, role)
    }
}

/// A driver built on the ledger, keeping every promise the contract states, walks the whole
/// lifecycle.
///
/// This is the control the other three are measured against: it is the reference implementation the
/// specification describes, so a suite that could not carry it through would be reporting on itself
/// rather than on a driver.
#[test]
fn a_ledger_built_driver_that_keeps_the_contract_passes_the_walk() -> Result<(), Box<dyn Error>> {
    run(
        UnmonitoredPanel::keeping_the_contract(),
        panel_scan("conformance-passing")?,
    )?;

    Ok(())
}

/// A driver that takes its lease and never files it is caught.
///
/// This is the defect the suite exists for, and the one nothing else can see. Every kernel reading
/// stays healthy through it: the same establishment is published, the same status, the same
/// presentation. The promise is broken only in the driver's own record, and the answer is identical
/// for a streaming driver and a still one — which is why this control carries no flow expectation
/// at all.
///
/// Where this deviates from the specification: the specification has this control failing at the
/// ledger level, when the walk's teardown asks for a release and `release_lease` answers
/// `NotEstablished` where `Released` was promised. It fails earlier than that, at
/// [`ConformanceStep::SessionEstablished`], because the walk asks the subject what its own record
/// holds the moment the kernel publishes an establishment — and a driver that dropped its lease
/// answers `NotEstablished` there, several steps before any teardown. Failing at the first step
/// that can see the defect is the better control, so the assertion below pins that step rather than
/// the one the specification named.
#[test]
fn a_driver_that_never_files_its_lease_fails_the_walk() -> Result<(), Box<dyn Error>> {
    let Err(failure) = run(
        UnmonitoredPanel::never_filing_its_lease(),
        panel_scan("conformance-leaseless")?,
    ) else {
        return Err("a driver that never filed its lease was reported conformant".into());
    };

    assert_eq!(
        failure.stop(),
        ConformanceStop::At(ConformanceStep::SessionEstablished),
        "the lease-less driver failed somewhere other than establishment: {failure:?}"
    );
    assert!(
        matches!(
            failure.refusal(),
            ConformanceRefusal::SubjectRecordContradictsTheKernel { .. }
        ),
        "the lease-less driver was failed by a kernel reading rather than by its own record: \
         {failure:?}"
    );
    assert!(
        failure.reached().reached(ConformanceStep::AttemptIssued),
        "the lease-less driver was stopped before it was ever handed an attempt: {failure:?}"
    );
    Ok(())
}

/// A driver that ignores the kernel's cancellation is caught at the first cancelled attempt.
///
/// This is the regression for a walk that read only kernel state. `cancel_apply` is dispatched
/// whether the driver acts on it or not, and every reading the kernel publishes afterwards — the
/// role status, the retained role entity, the attempt reference it stopped naming — is identical
/// for a driver that unwound the attempt and one whose body is empty. The only evidence that
/// separates them is the driver's own record of having been told, so that is what the walk's
/// cancellation steps are asserted against.
///
/// This control keeps every other promise: it files its lease, it releases its sessions, and it
/// reaches every step through retirement. It stops at the walk's first cancelled attempt, which is
/// the earliest point the missing body can be seen at all.
#[test]
fn a_driver_that_ignores_a_cancelled_attempt_fails_the_walk() -> Result<(), Box<dyn Error>> {
    let Err(failure) = run(
        UnmonitoredPanel::never_unwinding_a_cancellation(),
        panel_scan("conformance-ignoring-cancellation")?,
    ) else {
        return Err("a driver whose `cancel_apply` has no body was reported conformant".into());
    };

    assert_eq!(
        failure.stop(),
        ConformanceStop::At(ConformanceStep::ReplacementCancelledTheAttempt),
        "the ignoring driver failed somewhere other than the walk's first cancelled attempt: \
         {failure:?}"
    );
    assert!(
        matches!(
            failure.refusal(),
            ConformanceRefusal::CleanupNeverReachedTheSubject { .. }
        ),
        "the ignoring driver was failed by a kernel reading rather than by the cleanup verb never \
         reaching it: {failure:?}"
    );
    assert!(
        failure
            .reached()
            .reached(ConformanceStep::RetirementEndedTheSession),
        "the ignoring driver was stopped before it had released a session, so its cancellation was \
         never the thing under test: {failure:?}"
    );
    Ok(())
}

/// A subject that promises continuous data and delivers some keeps its session through the walk.
///
/// Without this the stall control below would be satisfied by a suite that failed every monitored
/// subject on principle, which would prove nothing about flow.
#[test]
fn a_streaming_driver_that_keeps_delivering_passes_the_walk() -> Result<(), Box<dyn Error>> {
    run(
        StreamingPanel::delivering(),
        panel_scan("conformance-flow")?,
    )?;

    Ok(())
}

/// A subject that promises continuous data and delivers none is failed on that promise.
///
/// The flow step runs only for a monitored subject: an unmonitored driver never promised a datum,
/// and failing it for silence would be the suite inventing a contract the driver never signed.
///
/// What this asserts, and why it is not "the walk ran out of frames": a step that failed because a
/// reading never appeared would report the same thing for a driver that stalled and for a walk
/// whose budget was too short, and it would keep reporting it if the kernel's flow judgement were
/// deleted outright. The refusal below is raised from the kernel's own stall verdict, observed as
/// `RolePresentationView::Connected(ConnectedCause::Stalled(cause))` on the role's published
/// presentation, and the cause it carries names which declared bound was crossed. Delete the flow
/// judgement and that reading never appears, the refusal never fires, and this control goes red —
/// which is what makes it evidence that the declared bound is really enforced.
#[test]
fn a_streaming_driver_that_delivers_no_datum_fails_the_flow_step() -> Result<(), Box<dyn Error>> {
    let Err(failure) = run(StreamingPanel::silent(), panel_scan("conformance-stall")?) else {
        return Err("a streaming driver that delivered no datum was reported conformant".into());
    };

    assert_eq!(
        failure.stop(),
        ConformanceStop::At(ConformanceStep::FlowRead),
        "the silent streamer failed somewhere other than its flow: {failure:?}"
    );
    assert!(
        matches!(
            failure.refusal(),
            ConformanceRefusal::FlowStalledBeforeItPresented { .. }
        ),
        "the silent streamer was failed by an exhausted frame budget rather than by the kernel's \
         own stall judgement: {failure:?}"
    );
    assert!(
        failure
            .reached()
            .reached(ConformanceStep::SessionEstablished),
        "the silent streamer never established, so its flow was never the thing under test: \
         {failure:?}"
    );
    Ok(())
}

/// The process-local driver route the panel's own plugin issued.
///
/// A shipped driver publishes exactly this — `ScreenDriverId` and `CameraDriverId` are the two in
/// the repo — so the caller that authors bindings reads the route back out of the world rather
/// than being handed it. Every fake here goes through the same resource, because `install`'s
/// contract is that the subject's plugin registers the driver and `install` returns what the plugin
/// produced; a fake calling `add_endpoint_driver` inline would leave that contract untested.
#[derive(Resource, Clone, Copy)]
struct PanelDriverId(EndpointDriverRegistration<PanelPlacement>);

/// The panel driver's own plugin: what registers the driver and adds its per-frame work.
///
/// Generic over the driver because the two fakes that reach the walk differ only in how they
/// resolve their target, and a second plugin whose body was a copy of this one would be testing
/// the copy. `build` is handed only `&self`, so the driver arrives as a `fn` item rather than a
/// value the plugin could move out of itself.
struct ScriptedPanelPlugin<Driver> {
    state:  Arc<Mutex<FakePanelState>>,
    flow:   FlowDiscipline,
    driver: fn(Arc<Mutex<FakePanelState>>) -> Driver,
}

impl<Driver> Plugin for ScriptedPanelPlugin<Driver>
where
    Driver: EndpointDriver<Configuration = PanelPlacement>,
{
    fn build(&self, app: &mut App) {
        let registration = app.add_endpoint_driver((self.driver)(Arc::clone(&self.state)));
        app.insert_resource(PanelDriverId(registration));
        match self.flow {
            FlowDiscipline::Crediting => {
                app.insert_resource(CreditingPanel(Arc::clone(&self.state)))
                    .add_systems(Update, credit_the_authored_role);
            },
            FlowDiscipline::Silent => {},
        }
    }
}

/// A subject whose target needs its declared capability and its attached role entity walks too.
///
/// Every other control here resolves its target unconditionally, which leaves two clauses of the
/// subject contract asserted by nothing: that `declare`'s capability reaches the presented device,
/// and that `attach_role` reaches the role entity before its binding's first frame. This driver
/// defers without either, so a walk that reaches an attempt has proven both.
#[test]
fn a_driver_that_waits_on_its_capability_and_its_attachment_passes_the_walk()
-> Result<(), Box<dyn Error>> {
    let report = run(AttachingPanel::new(), panel_scan("conformance-attaching")?)?;

    assert!(
        report.reached(ConformanceStep::AttemptIssued),
        "the attaching driver never reached an attempt, so neither its capability nor its \
         attachment arrived: {report:?}"
    );
    Ok(())
}

/// Add the subject's own plugin and hand back the registration that plugin produced.
///
/// This is the whole of what a subject's `install` owes the walk, and it is the round trip the
/// contract describes: the plugin registers the driver and publishes its route, and `install` reads
/// the route back. `Plugin::build` runs inside `add_plugins`, so the resource is published by the
/// time the next line reads it — the same read a shipped driver's caller makes of `ScreenDriverId`.
fn install_panel_plugin<Driver>(
    app: &mut App,
    state: &Arc<Mutex<FakePanelState>>,
    reporter: ReporterId,
    flow: FlowDiscipline,
    driver: fn(Arc<Mutex<FakePanelState>>) -> Driver,
) -> EndpointDriverRegistration<PanelPlacement>
where
    Driver: EndpointDriver<Configuration = PanelPlacement>,
{
    // Captured here for the same reason `ScreenDriver` captures its own at plugin build: a driver
    // reads every capability against the reporter that published it, and one it never captured is
    // a channel it can never use.
    lock(state).reporter = DeclaringReporter::Installed(reporter);
    app.add_plugins(ScriptedPanelPlugin {
        state: Arc::clone(state),
        flow,
        driver,
    });

    app.world().resource::<PanelDriverId>().0
}

/// One scripted scan reporting the named panel as present.
///
/// The suite synthesizes the departure and the return around this, so a case supplies only the set
/// its device is in — an empty scan written here would depart the panel before the walk began.
fn panel_scan(name: &str) -> Result<ScriptedScan, Box<dyn Error>> {
    Ok(scan![ScriptedDevice::present(panel_key(name)?)])
}

fn panel_key(value: &str) -> Result<DeviceKey, Box<dyn Error>> {
    Ok(reported_key(
        DeviceKind::ControlSurface,
        PANEL_SCHEME,
        value,
    )?)
}

/// The first-datum bound as a const, since the subject's flow expectation is an associated const.
///
/// The constructor is fallible and `const`, and `?`, `unwrap` and `expect` are all unavailable in a
/// const initializer, so the non-zero literal is checked here at compile time instead.
#[expect(
    clippy::panic,
    reason = "a const initializer cannot use `?`, `unwrap` or `expect`, so a `match` with a panic \
              is the only way to check a fallible const constructor at compile time"
)]
const fn first_datum_bound() -> FirstDatumTimeout {
    match FirstDatumTimeout::new(FIRST_DATUM_BOUND) {
        Ok(bound) => bound,
        Err(_) => panic!("the first-datum bound in this file is zero"),
    }
}

/// The datum-gap bound as a const, for the same reason as [`first_datum_bound`].
#[expect(
    clippy::panic,
    reason = "same as `first_datum_bound`: no fallible-constructor sugar exists in a const"
)]
const fn datum_gap_bound() -> MaximumDatumGap {
    match MaximumDatumGap::new(DATUM_GAP_BOUND) {
        Ok(bound) => bound,
        Err(_) => panic!("the datum-gap bound in this file is zero"),
    }
}

/// The bus line a device's declaration names.
///
/// Derived from the key so a declaration is about the device it was built for. Any stable
/// derivation would serve; the byte comes off the hash without a cast or a fallible conversion.
fn bus_line_for(device: &DeviceKey) -> u8 {
    let mut hasher = DefaultHasher::new();
    device.hash(&mut hasher);

    hasher.finish().to_le_bytes()[0]
}

/// Every cleanup verb this driver was dispatched for one role, oldest first.
///
/// Cumulative: the walk reads the whole record at each teardown step and looks for what that step
/// added, so a reader that drained would leave every later step blind to what came before it.
fn recorded_cleanups(state: &Arc<Mutex<FakePanelState>>, role: &RoleKey) -> RecordedCleanups {
    RecordedCleanups::new(
        lock(state)
            .cleanups
            .iter()
            .filter(|recorded| &recorded.role == role)
            .map(|recorded| recorded.cleanup.clone())
            .collect(),
    )
}

fn frame_instant(world: &World) -> Instant {
    let time = world.resource::<Time<Real>>();
    time.last_update().unwrap_or_else(|| time.startup())
}

fn lock(state: &Arc<Mutex<FakePanelState>>) -> MutexGuard<'_, FakePanelState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
