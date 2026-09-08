use std::time::Duration;

use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::world::EntityWorldMut;
use bevy::prelude::Add;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Time;
use bevy::prelude::Window;
use bevy::prelude::WindowPosition;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::debug;
use bevy::prelude::warn;
use bevy::window::PrimaryWindow;
use hana_kana::ToU32;
use hana_rigging::prelude::AvailableConfiguration;
use hana_rigging::prelude::BindingAuthoring;
use hana_rigging::prelude::BindingError;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::DeviceEndpoint;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::HardwareWait;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleKeyError;
use hana_rigging::prelude::RoleStatus;
use hana_rigging::prelude::RoleStatusView;
use hana_rigging::prelude::SchemeName;
use hana_rigging::prelude::WaitTiming;
use hana_rigging::prelude::WaitingStatusView;

use crate::Platform;
use crate::WindowRevealDisposition;
use crate::constants::EXACT_DISPLAY_WAIT_TIMEOUT_SECS;
use crate::deadline::OperatingSystemWorkDeadline;
use crate::deadline::OperatingSystemWorkDeadlineStatus;
use crate::driver::WindowDriverId;
use crate::managed::ManagedWindow;
use crate::managed::ManagedWindowName;
use crate::managed::WindowRiggingRole;
use crate::monitors::CurrentMonitor;
use crate::monitors::DisplayFingerprint;
use crate::monitors::DisplayIdentity;
use crate::monitors::DisplayTopologyObservation;
use crate::monitors::EnumeratedDisplayEvidence;
use crate::monitors::MonitorConnected;
use crate::persistence;
use crate::persistence::EstablishedWindowPlacement;
use crate::persistence::PersistedWindowPlacementLookup;
use crate::persistence::PersistedWindowPlacements;
use crate::persistence::PersistedWindowTargetV5;
use crate::persistence::RestorableWindowPosition;
use crate::platform;
use crate::recovery::StrandedWindowMovementBaselines;
use crate::recovery::WindowFallbackRecoveryState;
use crate::reporter;
use crate::reporter::DisplayKeyClassification;

/// Prevents `hide_window_on_creation` from hiding a recovery shell.
#[derive(Component)]
pub(crate) struct SkipInitialWindowHide;

/// Hide the primary window when created, before winit creates the OS window.
///
/// Uses an observer on `PrimaryWindow` component addition, so it works regardless
/// of plugin order. The window will be shown after restore completes or after the
/// bounded reveal wait if no saved state exists.
///
/// The observer watches `Add<PrimaryWindow>` rather than `Add<Window>`: when `Window` is
/// added, `PrimaryWindow` may not exist yet, while a `PrimaryWindow` addition means the
/// `Window` component is already on the entity.
pub(crate) fn hide_window_on_creation(
    add: On<Add, PrimaryWindow>,
    mut commands: Commands,
    mut windows: Query<&mut Window, Without<SkipInitialWindowHide>>,
) {
    debug!(
        "[hide_window_on_creation] Observer fired for entity {:?}",
        add.entity
    );
    if let Ok(mut window) = windows.get_mut(add.entity) {
        debug!("[hide_window_on_creation] Setting window.visible = false");
        window.visible = false;
        commands
            .entity(add.entity)
            .remove::<WindowRevealDisposition>()
            .insert(SavedDisplayRevealWait::default());
    }
}

/// Bounded wait before fallback reveals one hidden managed window.
///
/// This component exists only for the current hidden lifetime. Each newly hidden managed window
/// receives a fresh timer, so one window's wait cannot reveal another or shorten a later respawn.
#[derive(Component)]
pub(crate) struct SavedDisplayRevealWait {
    timeout: OperatingSystemWorkDeadline,
}

impl Default for SavedDisplayRevealWait {
    fn default() -> Self {
        Self {
            timeout: OperatingSystemWorkDeadline::new(EXACT_DISPLAY_WAIT_TIMEOUT_SECS),
        }
    }
}

/// A managed window whose bounded placement wait ended with the kernel proving no placement was
/// coming, together with the display whose return restarts placement.
///
/// Folding this arm into fallback-and-return recovery does not close it.
/// `WindowFallbackRecoveryState` records a phase and nothing reads it outside tests, and
/// `hana_rigging`'s `apply_device_unavailability` records `WaitingWork::RestorationOwed` for
/// `RecoveryPolicy::ReapplyOnReturn` only when a readback has already established a value on the
/// endpoint. This arm is reached from a stopped role or a non-reporter wait, which is exactly the
/// never-established case, so that policy owes nothing here.
///
/// The state is left only through [`PlacementAbandoned::reenter_placement`], which consumes `self`
/// and returns the placing state, so no code path holds a placing state while the abandoned one is
/// still on the entity.
///
/// Reflected as a component because a window stuck abandoned on a display that never returns is
/// exactly what an operator inspects over BRP, and state that cannot be observed is not
/// instrumentation.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub(crate) struct PlacementAbandoned {
    wanted: WantedDisplay,
    since:  Duration,
}

impl PlacementAbandoned {
    /// Record that placement ended, naming the display whose return restarts it.
    pub(crate) const fn new(wanted: WantedDisplay, since: Duration) -> Self {
        Self { wanted, since }
    }

    /// Report when this abandonment happened.
    ///
    /// Two abandonments of the same window are otherwise indistinguishable, so a caller that must
    /// tell "never restarted" from "restarted, refused again, and abandoned a second time" reads
    /// this rather than the component's presence.
    #[cfg(test)]
    pub(crate) const fn since(&self) -> Duration { self.since }

    /// Re-enter placement under the same bound a first attempt receives.
    ///
    /// Consuming `self` is the transition: the returned wait is the placing state, and the
    /// abandoned state cannot outlive the call that produced it.
    #[allow(
        clippy::unused_self,
        reason = "the receiver is taken to consume the abandoned state, not to be read; an \
                  associated function would let a caller produce the placing state while the \
                  abandoned component is still on the entity"
    )]
    fn reenter_placement(self) -> SavedDisplayRevealWait { SavedDisplayRevealWait::default() }
}

/// The display an abandoned window is waiting for, and the evidence that recognises it on arrival.
///
/// Deliberately not `Option<DeviceKey>`. The absent case is not "no value" but "no durable
/// evidence names this display", and that case must never match an arrival; `Option`'s derived
/// equality would let two unnamed displays compare as one and restart a window on hardware it was
/// never saved to. The two named variants exist because the save format carries two kinds of
/// evidence: the current format writes a reporter key, while a record decoded from a v4 file holds
/// only a fingerprint until a live association converts it.
#[derive(Reflect)]
pub(crate) enum WantedDisplay {
    /// Reporter evidence already named the saved display with this exact kernel key. An arriving
    /// display matches when its own evidence classifies to the same key.
    Keyed(DeviceKey),
    /// Only a pre-v5 fingerprint names the saved display, because no live association has
    /// converted the record to a reporter key. An arriving display matches on the same fingerprint.
    Fingerprinted(DisplayFingerprint),
    /// No durable evidence names the saved display, so no arrival can establish that this display
    /// returned and the window stays abandoned.
    Unnamed,
}

impl WantedDisplay {
    /// Read the display that `role` is saved to.
    ///
    /// Only the persisted record answers. The binding's endpoint deliberately does not: when
    /// nothing is persisted, `device_for_current_monitor` authors that endpoint from
    /// `CurrentDisplay` evidence, so it names the fallback display the window is already sitting
    /// on rather than one it is waiting for. Keying the retry on it would hide and re-reveal a
    /// correctly placed window for a whole timeout every time that display was replugged.
    ///
    /// A role with no persisted record is therefore [`Self::Unnamed`]: nothing durable says which
    /// display this window belongs on, so no arrival can establish that it returned.
    fn for_role(persisted: &PersistedWindowPlacements, role: &RoleKey) -> Self {
        if let PersistedWindowPlacementLookup::Saved(placement) = persisted.get(role) {
            return match &placement.window_state.target {
                PersistedWindowTargetV5::Classified(device_key) => Self::Keyed(device_key.clone()),
                PersistedWindowTargetV5::AwaitingLegacyEvidence(legacy_identity) => {
                    match DisplayIdentity::from(*legacy_identity) {
                        DisplayIdentity::Fingerprinted(fingerprint) => {
                            Self::Fingerprinted(fingerprint)
                        },
                        DisplayIdentity::Anonymous => Self::Unnamed,
                    }
                },
            };
        }
        Self::Unnamed
    }

    /// Whether `arrival` is the display this window is waiting for.
    ///
    /// The keyed comparison runs the arriving display's own evidence back through
    /// [`classify_display_key`], the classifier the reporter itself uses to name a device, so the
    /// key compared here is built the same way rather than rebuilt to resemble it.
    ///
    /// An arrival whose evidence produces no key at all
    /// ([`DisplayKeyClassification::MatchEvidenceOnly`]) never matches: it is the arrival-side
    /// counterpart of [`Self::Unnamed`], and treating "carries no key" as a match would restart a
    /// window on whichever unidentifiable display happened to be plugged in.
    ///
    /// The [`Self::Fingerprinted`] arm is the one place a [`DisplayIdentity`] is compared, and it
    /// exists only for a record decoded from a v4 file that no live association has converted yet.
    /// It is not a second way to recognise a live display: every current record carries a reporter
    /// key and takes the [`Self::Keyed`] arm, and once `resolve_pending_legacy_targets` converts a
    /// retained record this arm stops being reachable for it.
    fn returned_as(
        &self,
        arrival: &EnumeratedDisplayEvidence,
        edid_serial_scheme: &SchemeName,
    ) -> DisplayArrival {
        match self {
            Self::Keyed(wanted) => {
                match platform::classify_display_key(&arrival.device_evidence, edid_serial_scheme) {
                    DisplayKeyClassification::Keyed(arrived) if arrived == *wanted => {
                        DisplayArrival::WantedDisplayReturned(arrived)
                    },
                    DisplayKeyClassification::Keyed(_)
                    | DisplayKeyClassification::MatchEvidenceOnly => DisplayArrival::OtherDisplay,
                }
            },
            Self::Fingerprinted(wanted) => {
                if arrival.legacy_identity == DisplayIdentity::Fingerprinted(*wanted) {
                    match platform::classify_display_key(
                        &arrival.device_evidence,
                        edid_serial_scheme,
                    ) {
                        DisplayKeyClassification::Keyed(arrived) => {
                            DisplayArrival::WantedDisplayReturned(arrived)
                        },
                        DisplayKeyClassification::MatchEvidenceOnly => {
                            DisplayArrival::WantedDisplayReturnedUnkeyed
                        },
                    }
                } else {
                    DisplayArrival::OtherDisplay
                }
            },
            Self::Unnamed => DisplayArrival::OtherDisplay,
        }
    }
}

/// What one arriving display is, judged against the display an abandoned window is waiting for.
///
/// The recognised variant carries the arrival's own kernel key rather than a flag, because
/// recognising the display and naming the endpoint to point the binding at are the same act: a
/// window is abandoned only after `rebind_window_to_its_current_display` has already moved its
/// binding onto whichever display the compositor opened it on, so restarting the role without the
/// returning key would place the window on that fallback display all over again.
enum DisplayArrival {
    /// The saved display returned and reporter evidence names it, so the binding can be pointed
    /// back at it.
    WantedDisplayReturned(DeviceKey),
    /// A pre-v5 fingerprint recognises the returning display but no reporter key names it, so the
    /// binding keeps the endpoint it has and the role is only restarted.
    WantedDisplayReturnedUnkeyed,
    /// Some other display arrived, or the arrival carries no evidence that could name it.
    OtherDisplay,
}

/// Restart placement for every abandoned window whose saved display just came back.
///
/// This observer is the only path out of [`PlacementAbandoned`]. There is no polling system: a
/// monitor arrival is already an event, and a system re-reading the topology every frame would
/// re-answer a question that only changes when this event fires.
///
/// The arriving display is read from [`DisplayTopologyObservation`], which `update_monitors`
/// installs in the same command closure that triggers [`MonitorConnected`], so it always describes
/// this arrival. `LiveDisplayEndpointLookup` would read more directly but must not be used here:
/// nothing orders `ClerestoryUpdateSet::MonitorTopology` against the reporter's collection, so
/// whether the kernel has published the arriving display's device key when this event fires
/// depends on schedule order, and the retry would match on some runs and never match on others.
///
/// A window whose [`WantedDisplay`] is [`WantedDisplay::Unnamed`] is never restarted, however many
/// displays arrive; nothing about an unidentifiable display establishes that it is the one the
/// window was saved to.
pub(crate) fn resume_placement_when_wanted_display_returns(
    connected: On<MonitorConnected>,
    observation: Res<DisplayTopologyObservation>,
    time: Res<Time>,
    persisted: Res<PersistedWindowPlacements>,
    driver: Res<WindowDriverId>,
    mut bindings: ResMut<Bindings>,
    window_rigging_roles: Query<&WindowRiggingRole>,
    role_keys: Query<&RoleKey>,
    abandoned_windows: Query<(Entity, &PlacementAbandoned), With<ManagedWindow>>,
    mut commands: Commands,
) {
    let DisplayTopologyObservation::Observed(observed) = observation.as_ref() else {
        return;
    };
    let Some(arrival) = observed
        .iter()
        .find(|evidence| evidence.entity == connected.entity)
    else {
        return;
    };
    let edid_serial_scheme = match reporter::edid_serial_scheme() {
        Ok(edid_serial_scheme) => edid_serial_scheme,
        Err(error) => {
            warn!(
                "[resume_placement_when_wanted_display_returns] invalid built-in EDID serial \
                 scheme: {error}"
            );
            return;
        },
    };
    for (window, abandoned) in &abandoned_windows {
        let returned = match abandoned.wanted.returned_as(arrival, &edid_serial_scheme) {
            DisplayArrival::OtherDisplay => continue,
            recognised => recognised,
        };
        debug!(
            "[resume_placement_when_wanted_display_returns] window {window} re-enters placement \
             after {:?} abandoned",
            time.elapsed().saturating_sub(abandoned.since)
        );
        resume_role_on_returned_display(
            &mut bindings,
            &driver,
            &persisted,
            &window_rigging_roles,
            &role_keys,
            window,
            &returned,
        );
        commands.entity(window).queue(reenter_abandoned_placement);
    }
}

/// Point one window's role back at the display that just returned, then let the kernel apply it.
///
/// Restarting the role alone is not enough. A window whose saved display was absent is opened by
/// the compositor on some other display, and `rebind_window_to_its_current_display` moves the
/// binding onto that fallback display with the geometry it read back there. By the time the
/// window is abandoned the binding names the wrong display and the wrong geometry, so a restart
/// would re-establish it exactly where it already sits. Only the persisted record still holds
/// what the window was saved with, which is why the replacement is authored from it.
///
/// [`DisplayArrival::WantedDisplayReturnedUnkeyed`] and a role with nothing persisted both fall
/// back to a plain restart: neither can name an endpoint to move to, and a restart is still what
/// gets a stopped role a fresh attempt.
fn resume_role_on_returned_display(
    bindings: &mut Bindings,
    driver: &WindowDriverId,
    persisted: &PersistedWindowPlacements,
    window_rigging_roles: &Query<&WindowRiggingRole>,
    role_keys: &Query<&RoleKey>,
    window: Entity,
    returned: &DisplayArrival,
) {
    let Ok(window_rigging_role) = window_rigging_roles.get(window) else {
        return;
    };
    let Ok(role) = role_keys.get(window_rigging_role.entity()) else {
        return;
    };
    let DisplayArrival::WantedDisplayReturned(device) = returned else {
        restart_role(bindings, role);
        return;
    };
    let PersistedWindowPlacementLookup::Saved(saved) = persisted.get(role) else {
        restart_role(bindings, role);
        return;
    };
    let Ok(existing) = bindings.binding(role) else {
        restart_role(bindings, role);
        return;
    };
    let authoring = BindingAuthoring::new(
        role.clone(),
        DeviceEndpoint {
            device: device.clone(),
            id:     existing.endpoint.id.clone(),
        },
        driver.0,
        EstablishedWindowPlacement::from(&saved.window_state),
        existing.policy(),
    );
    if let Err(error) = bindings.replace_authoring(authoring) {
        warn!(
            "[resume_placement_when_wanted_display_returns] could not point role {role} back at \
             its returning display: {error}"
        );
        restart_role(bindings, role);
    }
}

/// Ask the kernel to attempt this window's role again now that its display is back.
///
/// Re-entering the wait is not enough on its own. The role reached
/// [`ProjectedRolePlacementStatus::PlacementImpossible`] as a stopped role or a non-reporter wait,
/// and the kernel dispatches no work for either when a device returns:
/// `observe_stopped_role_endpoint` returns early for an `Unsupported` stop, and nothing else
/// re-arms a role that was never established. Without this call the window would hide, wait out a
/// fresh deadline against the same refusal, and be abandoned a second time.
///
/// A role that is not stopped needs no restart and reports `RoleNotStopped`, which is the ordinary
/// outcome for the non-reporter wait case rather than a failure.
fn restart_role(bindings: &mut Bindings, role: &RoleKey) {
    match bindings.restart_role(role) {
        Ok(()) | Err(BindingError::RoleNotStopped { .. }) => {},
        Err(error) => {
            warn!(
                "[resume_placement_when_wanted_display_returns] could not restart role {role}: \
                 {error}"
            );
        },
    }
}

/// Swap one window's abandoned state for a fresh placing state.
///
/// Written as an entity command because the transition consumes the abandoned component:
/// [`PlacementAbandoned::reenter_placement`] takes `self`, which a query borrow cannot supply.
/// [`WindowRevealDisposition`] is removed with it, so the window re-enters the deadline machinery
/// from the top exactly as a newly hidden window does.
fn reenter_abandoned_placement(mut window: EntityWorldMut) {
    let Some(abandoned) = window.take::<PlacementAbandoned>() else {
        return;
    };
    if let Some(mut pane) = window.get_mut::<Window>() {
        pane.visible = false;
    }
    window.remove::<WindowRevealDisposition>();
    window.insert(abandoned.reenter_placement());
}

enum AuthoredWindowRole<'a> {
    Primary,
    Managed(&'a str),
}

impl AuthoredWindowRole<'_> {
    fn role(&self) -> Result<RoleKey, RoleKeyError> {
        match self {
            Self::Primary => persistence::primary_window_role(),
            Self::Managed(name) => persistence::managed_window_role(name),
        }
    }
}

enum CurrentMonitorAvailability<'a> {
    Available(&'a CurrentMonitor),
    Unavailable,
}

impl<'a> From<Option<&'a CurrentMonitor>> for CurrentMonitorAvailability<'a> {
    fn from(current_monitor: Option<&'a CurrentMonitor>) -> Self {
        current_monitor.map_or(Self::Unavailable, Self::Available)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EstablishedConfigurationEvidence {
    Known,
    NeverEstablished,
}

enum SavedGeometryPlacementOutcome {
    Fitted,
    Unavailable,
}

/// Role-status meaning used when a hidden saved window reaches its placement deadline.
enum ProjectedRolePlacementStatus {
    /// No role entity or status projection exists for the managed window.
    NotProjected,
    /// Kernel work can still place the window without fallback recovery.
    PlacementInProgress,
    /// The reporter wait crossed the kernel's published bound.
    ReporterBoundCrossed(ReporterBoundCrossing),
    /// Application policy disabled the reporter, so no placement bound is running.
    ReporterDisabled,
    /// Departure or confirmed absence prevents exact placement.
    ExactDisplayUnavailable,
    /// A non-reporter wait or stopped role cannot produce exact placement.
    PlacementImpossible,
}

/// Reporter wait whose published bound has crossed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReporterBoundCrossing {
    AwaitingFirstReport,
    Unconfirmed,
    Unreachable,
}

impl From<&RoleStatus> for ProjectedRolePlacementStatus {
    fn from(role_status: &RoleStatus) -> Self {
        match role_status.view() {
            RoleStatusView::Applying { .. }
            | RoleStatusView::Established { .. }
            | RoleStatusView::Waiting(WaitingStatusView::KernelRetry { .. }) => {
                Self::PlacementInProgress
            },
            RoleStatusView::Waiting(WaitingStatusView::Reporter(
                HardwareWait::AwaitingFirstReport { timing, .. },
            )) => {
                reporter_wait_placement_status(timing, ReporterBoundCrossing::AwaitingFirstReport)
            },
            RoleStatusView::Waiting(WaitingStatusView::Reporter(HardwareWait::Unconfirmed {
                timing,
                ..
            })) => reporter_wait_placement_status(timing, ReporterBoundCrossing::Unconfirmed),
            RoleStatusView::Waiting(WaitingStatusView::Reporter(HardwareWait::Unreachable {
                timing,
                ..
            })) => reporter_wait_placement_status(timing, ReporterBoundCrossing::Unreachable),
            RoleStatusView::Waiting(WaitingStatusView::Reporter(
                HardwareWait::DepartureGrace { .. } | HardwareWait::Absent { .. },
            )) => Self::ExactDisplayUnavailable,
            RoleStatusView::Waiting(_) | RoleStatusView::Stopped(_) => Self::PlacementImpossible,
        }
    }
}

const fn reporter_wait_placement_status(
    timing: &WaitTiming,
    reporter_bound_crossing: ReporterBoundCrossing,
) -> ProjectedRolePlacementStatus {
    match timing {
        WaitTiming::Bounded { .. } => ProjectedRolePlacementStatus::PlacementInProgress,
        WaitTiming::Overdue { .. } => {
            ProjectedRolePlacementStatus::ReporterBoundCrossed(reporter_bound_crossing)
        },
        WaitTiming::Unbounded { .. } => ProjectedRolePlacementStatus::ReporterDisabled,
    }
}

/// Conclude placement for a managed window when its bounded wait proves no placement will arrive.
///
/// `place_window_at_saved_geometry` is the normal production path that records the saved geometry,
/// and it runs only once the kernel resolves the role's endpoint to a live display. A role with no
/// saved configuration never enters that path. A saved display that is absent resolves to nothing —
/// whether `author_window_bindings` declined to author a binding at all, or authored one naming
/// that display and left the role waiting on a device that will never arrive. Either way no attempt
/// is issued, so the deadline records why no normal placement will arrive.
///
/// Asking whether a binding exists is therefore the wrong question; the question is whether a live
/// display can satisfy the one this role owns. When none can, the saved geometry is fitted onto the
/// display the window launched on.
///
/// The persisted record and the binding both keep naming the absent display, so an opted-in return
/// policy can still move the window back to it when that display returns.
/// `managed::adopt_live_display_for_stranded_window` changes the saved target only if the user
/// moves the window first.
pub(crate) fn abandon_placement_after_deadline(
    time: Res<Time>,
    bindings: Res<Bindings>,
    persisted: Res<PersistedWindowPlacements>,
    window_rigging_roles: Query<&WindowRiggingRole>,
    role_statuses: Query<&RoleStatus>,
    platform: Res<Platform>,
    mut fallback: ResMut<WindowFallbackRecoveryState>,
    mut stranded: ResMut<StrandedWindowMovementBaselines>,
    mut windows: Query<
        (
            Entity,
            &mut Window,
            Option<&CurrentMonitor>,
            Has<PrimaryWindow>,
            Option<&ManagedWindowName>,
            &mut SavedDisplayRevealWait,
        ),
        With<ManagedWindow>,
    >,
    mut commands: Commands,
) {
    for (entity, mut window, current_monitor, primary, managed_name, mut wait) in &mut windows {
        if window.visible {
            commands.entity(entity).remove::<SavedDisplayRevealWait>();
            continue;
        }
        let role_source = if primary {
            AuthoredWindowRole::Primary
        } else {
            let Some(managed_name) = managed_name else {
                commands.entity(entity).remove::<SavedDisplayRevealWait>();
                continue;
            };
            AuthoredWindowRole::Managed(&managed_name.0)
        };
        let role = match role_source.role() {
            Ok(role) => role,
            Err(error) => {
                warn!(
                    "[abandon_placement_after_deadline] managed window {entity} has an invalid role: {error}"
                );
                commands.entity(entity).remove::<SavedDisplayRevealWait>();
                continue;
            },
        };
        if matches!(
            wait.timeout.advance(time.delta()),
            OperatingSystemWorkDeadlineStatus::Pending
        ) {
            continue;
        }
        match established_configuration_evidence(&bindings, &persisted, &role) {
            EstablishedConfigurationEvidence::NeverEstablished => {
                debug!(
                    "[abandon_placement_after_deadline] role {role} has no saved \
                         configuration; keeping its opening position after \
                         {EXACT_DISPLAY_WAIT_TIMEOUT_SECS}s"
                );
                commands
                    .entity(entity)
                    .insert(WindowRevealDisposition::NothingSaved)
                    .remove::<(SavedDisplayRevealWait, PlacementAbandoned)>();
                continue;
            },
            EstablishedConfigurationEvidence::Known => {},
        }
        let projected_role_status = window_rigging_roles.get(entity).map_or(
            ProjectedRolePlacementStatus::NotProjected,
            |window_rigging_role| {
                role_statuses
                    .get(window_rigging_role.entity())
                    .map_or(ProjectedRolePlacementStatus::NotProjected, Into::into)
            },
        );
        if handle_kernel_status_at_deadline(
            &mut commands,
            entity,
            &role,
            projected_role_status,
            &persisted,
            time.elapsed(),
        ) {
            continue;
        }
        warn!(
            "[abandon_placement_after_deadline] no live display can satisfy the saved target of \
             role {role} after {EXACT_DISPLAY_WAIT_TIMEOUT_SECS}s"
        );
        let disposition = fit_to_fallback_display(
            &bindings,
            &persisted,
            role,
            current_monitor,
            *platform,
            &mut window,
            &mut fallback,
            &mut stranded,
        );
        commands
            .entity(entity)
            .insert(disposition)
            .remove::<(SavedDisplayRevealWait, PlacementAbandoned)>();
    }
}

/// Handle a projected kernel status that either extends the wait or proves placement impossible.
///
/// Returns `true` when the caller should stop processing this window for the current frame. A
/// reporter wait crosses into fallback only through [`ReporterBoundCrossing`].
///
/// The [`ProjectedRolePlacementStatus::PlacementImpossible`] arm is the one conclusion that used to
/// end placement for the life of the process, so it records [`PlacementAbandoned`] naming the
/// display whose return restarts it. A display that leaves and comes back is the normal case for
/// a projector rig, and before this arm recorded anything the conclusion was terminal for the life
/// of the process: the window was revealed where it opened and nothing watched for the display's
/// return, so a replugged projector showed a black output while every component in the main world
/// read as correct.
///
/// The two abandoning returns of [`fit_to_fallback_display`] do
/// not: they already call `WindowFallbackRecoveryState::mark_missing` and
/// `StrandedWindowMovementBaselines::begin`, and they are reached from an established role that
/// `RecoveryPolicy::ReapplyOnReturn` does bring back.
fn handle_kernel_status_at_deadline(
    commands: &mut Commands,
    entity: Entity,
    role: &RoleKey,
    projected_role_status: ProjectedRolePlacementStatus,
    persisted: &PersistedWindowPlacements,
    since: Duration,
) -> bool {
    match projected_role_status {
        ProjectedRolePlacementStatus::NotProjected
        | ProjectedRolePlacementStatus::ReporterDisabled
        | ProjectedRolePlacementStatus::ExactDisplayUnavailable => false,
        ProjectedRolePlacementStatus::ReporterBoundCrossed(reporter_bound_crossing) => {
            debug!(
                ?reporter_bound_crossing,
                %role,
                "reporter wait crossed its bound; saved display fallback is available"
            );
            false
        },
        ProjectedRolePlacementStatus::PlacementInProgress => true,
        ProjectedRolePlacementStatus::PlacementImpossible => {
            warn!(
                "[abandon_placement_after_deadline] kernel status confirms placement is not \
                 coming for role {role}"
            );
            commands
                .entity(entity)
                .insert((
                    WindowRevealDisposition::PlacementAbandoned,
                    PlacementAbandoned::new(WantedDisplay::for_role(persisted, role), since),
                ))
                .remove::<SavedDisplayRevealWait>();
            true
        },
    }
}

fn established_configuration_evidence(
    bindings: &Bindings,
    persisted: &PersistedWindowPlacements,
    role: &RoleKey,
) -> EstablishedConfigurationEvidence {
    if persisted.has_saved_configuration(role) {
        return EstablishedConfigurationEvidence::Known;
    }
    match bindings.binding(role) {
        Ok(binding) if binding.last_known_good().is_ok() => EstablishedConfigurationEvidence::Known,
        Ok(_) | Err(_) => EstablishedConfigurationEvidence::NeverEstablished,
    }
}

/// Draw one window back onto the display it launched on, once its own saved display proved absent.
///
/// Marking the role missing and starting its stranded baseline belong with the placement, not
/// after it: both describe the same conclusion, that this window is sitting somewhere it was never
/// saved to be, and an opted-in return policy reads them together when the saved display comes
/// back.
fn fit_to_fallback_display(
    bindings: &Bindings,
    persisted: &PersistedWindowPlacements,
    role: RoleKey,
    current_monitor: Option<&CurrentMonitor>,
    platform: Platform,
    window: &mut Window,
    fallback: &mut WindowFallbackRecoveryState,
    stranded: &mut StrandedWindowMovementBaselines,
) -> WindowRevealDisposition {
    let disposition = match CurrentMonitorAvailability::from(current_monitor) {
        CurrentMonitorAvailability::Available(current_monitor) => {
            match fit_saved_geometry_to_live_monitor(
                bindings,
                persisted,
                &role,
                current_monitor,
                platform,
                window,
            ) {
                SavedGeometryPlacementOutcome::Fitted => {
                    WindowRevealDisposition::FittedToFallbackDisplay
                },
                SavedGeometryPlacementOutcome::Unavailable => {
                    WindowRevealDisposition::PlacementAbandoned
                },
            }
        },
        CurrentMonitorAvailability::Unavailable => WindowRevealDisposition::PlacementAbandoned,
    };
    fallback.mark_missing(role.clone());
    stranded.begin(role);
    disposition
}

/// Put the window at its saved size and offset, drawn back inside the display it launched on.
///
/// Nothing here is persisted and the role's authored configuration is untouched: this is only
/// where the window sits until its own display returns. Window mode is left alone, so a saved
/// fullscreen record is shown windowed rather than driven through the fullscreen restore machine
/// on a display it was never sized for.
fn fit_saved_geometry_to_live_monitor(
    bindings: &Bindings,
    persisted: &PersistedWindowPlacements,
    role: &RoleKey,
    current_monitor: &CurrentMonitor,
    platform: Platform,
    window: &mut Window,
) -> SavedGeometryPlacementOutcome {
    let placement = if let Ok(configuration) = bindings.configuration_for(role) {
        let (AvailableConfiguration::LastKnownGood(configuration)
        | AvailableConfiguration::Requested(configuration)) = configuration;
        let Some(placement) = configuration
            .as_any()
            .downcast_ref::<EstablishedWindowPlacement>()
        else {
            return SavedGeometryPlacementOutcome::Unavailable;
        };
        placement.clone()
    } else {
        let PersistedWindowPlacementLookup::Saved(persisted) = persisted.get(role) else {
            return SavedGeometryPlacementOutcome::Unavailable;
        };
        EstablishedWindowPlacement::from(&persisted.window_state)
    };
    let monitor = &current_monitor.descriptor;
    let fitted = placement.fitted_to(monitor);
    window.resolution.set_physical_resolution(
        (f64::from(fitted.logical_size.x) * monitor.scale).to_u32(),
        (f64::from(fitted.logical_size.y) * monitor.scale).to_u32(),
    );
    if platform.position_available()
        && let RestorableWindowPosition::Restorable {
            physical_position, ..
        } = fitted.restorable_position(monitor)
    {
        window.position = WindowPosition::At(physical_position);
    }
    SavedGeometryPlacementOutcome::Fitted
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;

    use bevy::MinimalPlugins;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::Commands;
    use bevy::prelude::Entity;
    use bevy::prelude::IVec2;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::On;
    use bevy::prelude::PreStartup;
    use bevy::prelude::Query;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Time;
    use bevy::prelude::UVec2;
    use bevy::prelude::Update;
    use bevy::prelude::Window;
    use bevy::prelude::WindowPosition;
    use bevy::prelude::With;
    use bevy::prelude::World;
    use bevy::prelude::default;
    use bevy::time::TimeUpdateStrategy;
    use bevy::time::Virtual;
    use bevy::window::ExitCondition;
    use bevy::window::Monitor;
    use bevy::window::OnMonitor;
    use bevy::window::PrimaryWindow;
    use bevy::window::WindowMode;
    use bevy::window::WindowPlugin;
    use hana_rigging::prelude::Applied;
    use hana_rigging::prelude::ApplyContext;
    use hana_rigging::prelude::ApplyDeadline;
    use hana_rigging::prelude::AttachmentPath;
    use hana_rigging::prelude::AttemptInvalidation;
    use hana_rigging::prelude::AttemptRef;
    use hana_rigging::prelude::AuthoritativeReporterCoverage;
    use hana_rigging::prelude::AvailableConfiguration;
    use hana_rigging::prelude::BindingAuthoring;
    use hana_rigging::prelude::BindingPolicy;
    use hana_rigging::prelude::Bindings;
    use hana_rigging::prelude::Capabilities;
    use hana_rigging::prelude::Claim;
    use hana_rigging::prelude::CoveredDeviceIdentitySpace;
    use hana_rigging::prelude::DeviceAccessError;
    use hana_rigging::prelude::DeviceDescriptor;
    use hana_rigging::prelude::DeviceEndpoint;
    use hana_rigging::prelude::DeviceIdSource;
    use hana_rigging::prelude::DeviceKey;
    use hana_rigging::prelude::DeviceKind;
    use hana_rigging::prelude::DeviceRecord;
    use hana_rigging::prelude::DeviceReporter;
    use hana_rigging::prelude::DeviceScan;
    use hana_rigging::prelude::DiscoveryCadence;
    use hana_rigging::prelude::DiscoveryControl;
    use hana_rigging::prelude::DiscoveryWork;
    use hana_rigging::prelude::DriverCleanupRoleEntity;
    use hana_rigging::prelude::DriverCompletion;
    use hana_rigging::prelude::EndpointDriver;
    use hana_rigging::prelude::EndpointDriverRegistration;
    use hana_rigging::prelude::EndpointId;
    use hana_rigging::prelude::EstablishedContext;
    use hana_rigging::prelude::HardwareWait;
    use hana_rigging::prelude::LastKnownGoodConfiguration;
    use hana_rigging::prelude::LiveRoleChange;
    use hana_rigging::prelude::LiveRoleChanged;
    use hana_rigging::prelude::MainThreadDiscoveryJob;
    use hana_rigging::prelude::OnAbort;
    use hana_rigging::prelude::OnSessionLoss;
    use hana_rigging::prelude::PlatformDeviceHandle;
    use hana_rigging::prelude::Presence;
    use hana_rigging::prelude::RecoveryPolicy;
    use hana_rigging::prelude::ReportedAs;
    use hana_rigging::prelude::ReportedId;
    use hana_rigging::prelude::ReportedParent;
    use hana_rigging::prelude::ReportedSerial;
    use hana_rigging::prelude::ReporterActivation;
    use hana_rigging::prelude::ReporterCoverage;
    use hana_rigging::prelude::ReporterId;
    use hana_rigging::prelude::ReporterRegistration;
    use hana_rigging::prelude::RetryOn;
    use hana_rigging::prelude::RiggingAppExt;
    use hana_rigging::prelude::RiggingPlugin;
    use hana_rigging::prelude::RiggingSystems;
    use hana_rigging::prelude::RoleKey;
    use hana_rigging::prelude::RoleStatus;
    use hana_rigging::prelude::RoleStatusView;
    use hana_rigging::prelude::SchemeName;
    use hana_rigging::prelude::SessionRef;
    use hana_rigging::prelude::SessionReleaseCause;
    use hana_rigging::prelude::StoppedStatusView;
    use hana_rigging::prelude::TargetResolution;
    use hana_rigging::prelude::TargetResolutionContext;
    use hana_rigging::prelude::TargetWait;
    use hana_rigging::prelude::WaitTiming;
    use hana_rigging::prelude::WaitingStatusView;
    use hana_rigging::prelude::register_binding;
    use hana_rigging::prelude::replace_binding;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::PlacementAbandoned;
    use super::ReporterBoundCrossing;
    use super::SavedDisplayRevealWait;
    use super::WantedDisplay;
    use super::abandon_placement_after_deadline;
    use super::hide_window_on_creation;
    use crate::DisplayTestAdapter;
    use crate::DisplayTestDescriptor;
    use crate::DisplayTestDeviceKey;
    use crate::ManagedWindowPersistence;
    use crate::Platform;
    use crate::WindowManagerPlugin;
    use crate::WindowRevealDisposition;
    use crate::constants::EXACT_DISPLAY_WAIT_TIMEOUT_SECS;
    use crate::display_test_adapter::DisplayTestEnumeration;
    use crate::driver::RestoreRecord;
    use crate::driver::WindowDriverId;
    use crate::driver::WindowEndpointDriver;
    use crate::driver::WindowPlacementTarget;
    use crate::driver::WindowRoleDriverState;
    use crate::managed;
    use crate::managed::ManagedWindow;
    use crate::managed::ManagedWindowName;
    use crate::managed::ManagedWindowRegistry;
    use crate::managed::WindowBindingAuthoring;
    use crate::managed::WindowRiggingRole;
    use crate::monitors::CurrentMonitor;
    use crate::monitors::CurrentMonitorEntity;
    use crate::monitors::DisplayDeviceEvidence;
    use crate::monitors::DisplayFingerprint;
    use crate::monitors::DisplayIdentity;
    use crate::monitors::DisplayIdentityEvidence;
    use crate::monitors::DisplayTopologyObservation;
    use crate::monitors::EnumeratedDisplayEvidence;
    use crate::monitors::LiveDisplayEndpoint;
    use crate::monitors::LiveDisplayMonitor;
    use crate::monitors::MonitorDescriptor;
    use crate::output_proof;
    use crate::persistence;
    use crate::persistence::EstablishedWindowPlacement;
    use crate::persistence::EstablishedWindowPosition;
    use crate::persistence::PersistedDisplayIdentityV4;
    use crate::persistence::PersistedPosition;
    use crate::persistence::PersistedWindowPlacementLookup;
    use crate::persistence::PersistedWindowPlacements;
    use crate::persistence::PersistedWindowState;
    use crate::persistence::PersistedWindowTargetV5;
    use crate::persistence::SavedWindowMode;
    use crate::platform;
    use crate::recovery::StrandedWindowMovementBaselines;
    use crate::recovery::WindowFallbackRecoveryPhase;
    use crate::recovery::WindowFallbackRecoveryProgress;
    use crate::recovery::WindowFallbackRecoveryState;
    use crate::reporter;
    use crate::reporter::DisplayKeyClassification;
    use crate::restore::InjectedWinitWindows;
    use crate::restore::WindowRestoreAttempt;
    use crate::restore_window_config::RestoreWindowConfig;
    use crate::show_window_once_placement_settles;

    /// Fraction of the wait that must leave a hidden window hidden.
    const PARTIAL_WAIT_FRACTION: f32 = 0.5;
    /// Fraction of the wait elapsed during each silent-failure reconsideration round.
    const RECONSIDERATION_ROUND_FRACTION: f32 = 0.25;
    /// Number of clocked updates used to prove reconsideration is not a single dispatch.
    const RECONSIDERATION_ROUNDS: usize = 4;
    pub(crate) const SAVED_LOGICAL_HEIGHT: u32 = 600;
    pub(crate) const SAVED_LOGICAL_WIDTH: u32 = 800;
    pub(crate) const SAVED_WINDOW_OFFSET: IVec2 = IVec2::new(100, 100);

    /// Geometry standing in for the one live display in these tests.
    fn live_descriptor() -> MonitorDescriptor {
        MonitorDescriptor::for_current_enumeration(0, 1.0, IVec2::ZERO, UVec2::new(1_920, 1_080))
    }

    #[test]
    fn a_fingerprinted_wanted_display_with_an_evidence_only_arrival_returns_unkeyed()
    -> Result<(), String> {
        let fingerprint = DisplayFingerprint::from_evidence_bytes(b"v4 wanted display");
        let wanted = WantedDisplay::Fingerprinted(fingerprint);
        let arrival = EnumeratedDisplayEvidence {
            entity:          Entity::PLACEHOLDER,
            descriptor:      live_descriptor(),
            device_evidence: DisplayDeviceEvidence {
                identity_evidence:      DisplayIdentityEvidence::Unavailable {
                    serial: ReportedSerial::PlatformCannotReport,
                },
                platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
                attachment:             AttachmentPath::PlatformHasNoConcept,
            },
            legacy_identity: DisplayIdentity::Fingerprinted(fingerprint),
        };
        let scheme = reporter::edid_serial_scheme()
            .map_err(|error| format!("the built-in display scheme is invalid: {error}"))?;

        assert!(matches!(
            wanted.returned_as(&arrival, &scheme),
            super::DisplayArrival::WantedDisplayReturnedUnkeyed
        ));
        Ok(())
    }

    fn saved_placement() -> EstablishedWindowPlacement {
        EstablishedWindowPlacement {
            position:          EstablishedWindowPosition::Restorable {
                logical_offset: SAVED_WINDOW_OFFSET,
            },
            logical_size:      UVec2::new(SAVED_LOGICAL_WIDTH, SAVED_LOGICAL_HEIGHT),
            saved_window_mode: SavedWindowMode::Windowed,
        }
    }

    fn saved_window_state(device: DeviceKey) -> PersistedWindowState {
        PersistedWindowState {
            target:            PersistedWindowTargetV5::Classified(device),
            position:          PersistedPosition::MonitorOffset(SAVED_WINDOW_OFFSET),
            logical_width:     SAVED_LOGICAL_WIDTH,
            logical_height:    SAVED_LOGICAL_HEIGHT,
            saved_window_mode: SavedWindowMode::Windowed,
            app_name:          String::from("visibility-test"),
        }
    }

    #[derive(Default, Resource)]
    struct EndedAttemptEvents(usize);

    fn count_ended_attempt(
        live_role_changed: On<LiveRoleChanged>,
        mut events: ResMut<EndedAttemptEvents>,
    ) {
        if matches!(
            live_role_changed.change,
            LiveRoleChange::AttemptEnded { .. }
        ) {
            events.0 += 1;
        }
    }

    fn reveal_app() -> App {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<Bindings>()
            .init_resource::<PersistedWindowPlacements>()
            .init_resource::<WindowFallbackRecoveryState>()
            .init_resource::<StrandedWindowMovementBaselines>()
            .insert_resource(Platform::FIXTURE)
            .add_observer(crate::mark_primary_window_as_managed)
            .add_systems(
                Update,
                (
                    abandon_placement_after_deadline,
                    show_window_once_placement_settles,
                )
                    .chain()
                    .after(RiggingSystems::Apply),
            );
        app
    }

    /// An application holding one primary window and no saved configuration or binding.
    fn primary_window_app(visible: bool) -> App {
        let mut app = reveal_app();
        app.world_mut().spawn((
            Window {
                visible,
                ..default()
            },
            PrimaryWindow,
            SavedDisplayRevealWait::default(),
        ));
        app
    }

    /// Step one frame that carries `seconds` of virtual time.
    ///
    /// `TimeUpdateStrategy` is a resource, not a one-shot: left holding this duration it would make
    /// every later `app.update()` advance the same amount, and a freshly armed deadline would
    /// expire on the next frame rather than after its own wait. Restoring a zero step keeps the
    /// stepped time inside this call, so a test advances time only where it says it does.
    pub(crate) fn advance(app: &mut App, seconds: f32) {
        let duration = Duration::from_secs_f32(seconds);
        if app.world().contains_resource::<TimeUpdateStrategy>() {
            app.insert_resource(TimeUpdateStrategy::ManualDuration(duration));
            app.update();
            app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
            return;
        }
        app.world_mut().resource_mut::<Time>().advance_by(duration);
        app.update();
    }

    fn window_is_visible(app: &mut App) -> bool {
        let mut query = app
            .world_mut()
            .query_filtered::<&Window, With<PrimaryWindow>>();
        query
            .single(app.world())
            .expect("the application holds one primary window")
            .visible
    }

    fn entity_is_visible(app: &App, entity: Entity) -> bool {
        app.world()
            .get::<Window>(entity)
            .expect("the managed entity has a window")
            .visible
    }

    pub(crate) fn has_reveal_wait(app: &App, entity: Entity) -> bool {
        app.world().get::<SavedDisplayRevealWait>(entity).is_some()
    }

    fn assert_fallback_window_placement(app: &App, entity: Entity) -> Result<(), String> {
        let window = app
            .world()
            .get::<Window>(entity)
            .ok_or_else(|| String::from("the fallback window disappeared"))?;
        assert!(window.visible);
        assert_eq!(window.resolution.physical_width(), SAVED_LOGICAL_WIDTH);
        assert_eq!(window.resolution.physical_height(), SAVED_LOGICAL_HEIGHT);
        assert_eq!(window.position, WindowPosition::At(SAVED_WINDOW_OFFSET));
        Ok(())
    }

    fn assert_adapter_fallback_window_placement(app: &App, entity: Entity) -> Result<(), String> {
        let window = app
            .world()
            .get::<Window>(entity)
            .ok_or_else(|| String::from("the adapter fallback window disappeared"))?;
        assert!(window.visible);
        assert_eq!(window.resolution.physical_width(), 1);
        assert_eq!(window.resolution.physical_height(), 1);
        assert_eq!(window.position, WindowPosition::At(IVec2::ZERO));
        Ok(())
    }

    fn recovery_phase(app: &App) -> WindowFallbackRecoveryProgress {
        let role = persistence::primary_window_role().expect("the primary role key is well formed");
        recovery_phase_for_role(app, &role)
    }

    fn recovery_phase_for_role(app: &App, role: &RoleKey) -> WindowFallbackRecoveryProgress {
        app.world()
            .resource::<WindowFallbackRecoveryState>()
            .phase(role)
    }

    #[test]
    fn reveals_a_never_saved_primary_bound_to_a_live_display() -> Result<(), String> {
        let (mut app, primary_window) = bound_window_app(
            BoundDisplay::Live,
            BoundWindow::Primary,
            BoundConfigurationHistory::NeverSaved,
            RecoveryPolicy::ReapplyOnReturn,
        )?;

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        assert!(entity_is_visible(&app, primary_window));
        assert!(!has_reveal_wait(&app, primary_window));
        assert_eq!(
            recovery_phase(&app),
            WindowFallbackRecoveryProgress::NotRecovering
        );
        Ok(())
    }

    #[test]
    fn leaves_a_hidden_window_hidden_before_the_wait_expires() {
        let mut app = primary_window_app(false);

        advance(
            &mut app,
            EXACT_DISPLAY_WAIT_TIMEOUT_SECS * PARTIAL_WAIT_FRACTION,
        );

        assert!(!window_is_visible(&mut app));
        assert_eq!(
            recovery_phase(&app),
            WindowFallbackRecoveryProgress::NotRecovering
        );
    }

    /// Whether the display a bound role names is currently plugged in.
    #[derive(Clone, Copy)]
    pub(crate) enum BoundDisplay {
        Live,
        Absent,
        ReporterDisabled,
    }

    #[derive(Clone, Copy)]
    pub(crate) enum BoundWindow {
        Primary,
        Managed(&'static str),
    }

    impl BoundWindow {
        fn role(self) -> Result<RoleKey, String> {
            match self {
                Self::Primary => persistence::primary_window_role()
                    .map_err(|error| format!("failed to create the primary role: {error}")),
                Self::Managed(name) => persistence::managed_window_role(name)
                    .map_err(|error| format!("failed to create managed role: {error}")),
            }
        }

        fn spawn(self, world: &mut World, monitor: Entity) -> Entity {
            let entity = world
                .spawn((
                    Window {
                        visible: false,
                        ..default()
                    },
                    CurrentMonitor {
                        descriptor:            live_descriptor(),
                        effective_window_mode: WindowMode::Windowed,
                    },
                    CurrentMonitorEntity::new(monitor),
                    OnMonitor(monitor),
                ))
                .id();
            match self {
                Self::Primary => {
                    world.entity_mut(entity).insert(PrimaryWindow);
                },
                Self::Managed(name) => {
                    world
                        .entity_mut(entity)
                        .insert(ManagedWindowName(name.to_string()));
                },
            }
            world
                .entity_mut(entity)
                .insert(SavedDisplayRevealWait::default());
            entity
        }
    }

    #[derive(Clone, Copy)]
    pub(crate) enum BoundConfigurationHistory {
        LoadedAtStartup,
        NeverSaved,
        /// Safe readback established this binding after launch and before persistence ran.
        SavedDuringSession,
    }

    #[derive(Clone, Resource)]
    struct BoundDisplayReportState(Arc<Mutex<BoundDisplay>>);

    #[derive(Resource)]
    struct BoundDisplayReporterId(ReporterId);

    struct BoundDisplayReporter {
        state:        Arc<Mutex<BoundDisplay>>,
        saved_device: DeviceKey,
        live_device:  DeviceKey,
        monitor:      Entity,
    }

    impl DeviceReporter for BoundDisplayReporter {
        fn discover(&mut self) -> DiscoveryWork {
            let bound_display = match self.state.lock() {
                Ok(state) => *state,
                Err(poisoned) => *poisoned.into_inner(),
            };
            let records = match bound_display {
                BoundDisplay::Live => {
                    vec![present_display(self.saved_device.clone(), self.monitor)]
                },
                BoundDisplay::Absent => {
                    vec![present_display(self.live_device.clone(), self.monitor)]
                },
                BoundDisplay::ReporterDisabled => Vec::new(),
            };
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(move |_: &mut World| {
                DeviceScan::Complete(records)
            }))
        }
    }

    struct PresentDisplayReporter {
        device:  DeviceKey,
        monitor: Entity,
    }

    impl DeviceReporter for PresentDisplayReporter {
        fn discover(&mut self) -> DiscoveryWork {
            let record = present_display(self.device.clone(), self.monitor);
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(move |_: &mut World| {
                DeviceScan::Complete(vec![record])
            }))
        }
    }

    struct BoundDisplayWindowDriver {
        bound_display:  BoundDisplay,
        apply_progress: BoundApplyProgress,
        reporter:       ReporterId,
    }

    #[derive(Clone, Copy)]
    enum BoundApplyProgress {
        Pending,
        Succeeded,
        Unsupported,
    }

    impl EndpointDriver for BoundDisplayWindowDriver {
        type Configuration = EstablishedWindowPlacement;
        type Target = ();

        fn resolve_target(
            &mut self,
            _: &mut World,
            _: &TargetResolutionContext<'_>,
            _: &Self::Configuration,
        ) -> TargetResolution<Self::Target> {
            match self.bound_display {
                BoundDisplay::Live => TargetResolution::Reached(()),
                BoundDisplay::Absent | BoundDisplay::ReporterDisabled => {
                    TargetResolution::Deferred(TargetWait::Reporter {
                        reporter: self.reporter,
                        error:    DeviceAccessError::Absent {
                            detail: String::from("the test display is unavailable"),
                        },
                    })
                },
            }
        }

        fn start_apply(
            &mut self,
            _: &mut World,
            context: ApplyContext<'_, Self::Configuration>,
            _: &Self::Configuration,
            (): Self::Target,
        ) {
            match self.apply_progress {
                BoundApplyProgress::Pending => {},
                BoundApplyProgress::Succeeded => context
                    .into_completion()
                    .finish(DriverCompletion::Succeeded(Applied::AsDispatched)),
                BoundApplyProgress::Unsupported => context.into_completion().finish(
                    DriverCompletion::Failed(DeviceAccessError::Unsupported {
                        detail: String::from("the test window placement is unsupported"),
                    }),
                ),
            }
        }

        fn established(&mut self, _: &mut World, _: EstablishedContext<'_, Self::Configuration>) {}

        fn cancel_apply(
            &mut self,
            _: &mut World,
            _: &RoleKey,
            _: DriverCleanupRoleEntity,
            _: AttemptRef,
            _: AttemptInvalidation,
        ) {
        }

        fn release_session(
            &mut self,
            _: &mut World,
            _: &RoleKey,
            _: DriverCleanupRoleEntity,
            _: SessionRef,
            _: SessionReleaseCause,
        ) {
        }
    }

    fn present_display(device: DeviceKey, monitor: Entity) -> DeviceRecord {
        DeviceRecord {
            reported_as:            ReportedAs::Keyed(device),
            parent:                 ReportedParent::Root,
            presence:               Presence::Present,
            claim:                  Claim::NotApplicable,
            capabilities:           Capabilities::new()
                .with(LiveDisplayEndpoint {
                    monitor,
                    descriptor: live_descriptor(),
                    legacy_identity: DisplayIdentity::Anonymous,
                })
                .with(LiveDisplayMonitor::new(monitor)),
            serial:                 ReportedSerial::NotExposedByUnit,
            platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
            attachment:             AttachmentPath::PlatformHasNoConcept,
            descriptor:             DeviceDescriptor::PlatformHasNoConcept,
        }
    }

    fn test_display_devices() -> Result<(SchemeName, DeviceKey, DeviceKey), String> {
        let scheme_name = SchemeName::new("reveal-test-display")
            .map_err(|error| format!("failed to create the test device scheme: {error}"))?;
        let saved_device = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Reported {
                scheme: scheme_name.clone(),
                value:  ReportedId::new("reveal-guard-display")
                    .map_err(|error| format!("failed to create the test device id: {error}"))?,
            },
        };
        let live_device = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Reported {
                scheme: scheme_name.clone(),
                value:  ReportedId::new("reveal-live-display")
                    .map_err(|error| format!("failed to create live device id: {error}"))?,
            },
        };
        Ok((scheme_name, saved_device, live_device))
    }

    fn unbound_saved_primary_window_app() -> Result<(App, Entity), String> {
        let mut app = reveal_app();
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create the primary role: {error}"))?;
        let (_, saved_device, _) = test_display_devices()?;
        app.world_mut()
            .resource_mut::<PersistedWindowPlacements>()
            .seed(HashMap::from([(role, saved_window_state(saved_device))]));
        let primary_window = app
            .world_mut()
            .spawn((
                Window {
                    visible: false,
                    ..default()
                },
                PrimaryWindow,
                CurrentMonitor {
                    descriptor:            live_descriptor(),
                    effective_window_mode: WindowMode::Windowed,
                },
                SavedDisplayRevealWait::default(),
            ))
            .id();
        Ok((app, primary_window))
    }

    fn settle_bound_window_app(app: &mut App, configuration_history: BoundConfigurationHistory) {
        let update_count = match configuration_history {
            BoundConfigurationHistory::SavedDuringSession => 8,
            BoundConfigurationHistory::LoadedAtStartup | BoundConfigurationHistory::NeverSaved => 3,
        };
        for _ in 0..update_count {
            app.update();
        }
    }

    fn report_bound_display(app: &mut App, bound_display: BoundDisplay) -> Result<(), String> {
        let state = Arc::clone(&app.world().resource::<BoundDisplayReportState>().0);
        match state.lock() {
            Ok(mut state) => *state = bound_display,
            Err(poisoned) => *poisoned.into_inner() = bound_display,
        }
        let reporter = app.world().resource::<BoundDisplayReporterId>().0;
        app.world_mut()
            .resource_mut::<DiscoveryControl>()
            .request(reporter)
            .map_err(|error| format!("failed to request the changed display report: {error}"))?;
        for _ in 0..3 {
            app.update();
        }
        Ok(())
    }

    /// Which abandoned window a placement-retry fixture builds.
    ///
    /// The two scripts differ in the evidence the saved record carries, which is the only thing
    /// [`WantedDisplay::for_role`] reads, and so the only input to whether any arrival
    /// can restart this window.
    pub(crate) enum AbandonedPlacementScript {
        /// The window is saved against a display reporter evidence can name, so a matching arrival
        /// restarts placement.
        WantedDisplayDeparted,
        /// The window's saved record carries no durable display evidence, so nothing an arrival
        /// reports can establish that this window's display returned.
        NoUsableKey,
    }

    /// Which display a placement-retry fixture reports arriving.
    pub(crate) enum RetryDisplayArrival {
        /// The display the window is saved against.
        WantedDisplay,
        /// A display carrying its own distinct serial, which classifies to a different
        /// [`DeviceKey`] than the saved one. Reusing the wanted display's evidence here would make
        /// every arrival match and stop the negative cases from testing anything.
        UnrelatedDisplay,
        /// A display whose scan produced no durable identity at all, so it classifies to
        /// [`DisplayKeyClassification::MatchEvidenceOnly`] and names no [`DeviceKey`].
        ///
        /// Such an arrival can resemble the wanted display in every descriptor a fallback fit
        /// reads and still be a different panel, which is why an unkeyed classification refuses
        /// rather than matching on resemblance.
        EvidenceOnlyLookalike,
    }

    /// What a placement-retry test needs to reach after its app is built.
    ///
    /// The adapter is the display platform: it holds the list of enumerated displays, and every
    /// departure and return in these tests is one call on it. The device key is the reporter's own
    /// name for the wanted display, so a match is never a resemblance between descriptors. The
    /// temporary directory is held because the window-state file must outlive the app.
    #[derive(Resource)]
    struct PlacementRetryFixture {
        adapter:      DisplayTestAdapter,
        wanted:       DeviceKey,
        departure:    WantedDisplayDeparture,
        #[allow(
            dead_code,
            reason = "the window state file must outlive the app, so the directory is held rather \
                      than read"
        )]
        window_state: TempDir,
    }

    /// Whether the wanted display has been observed leaving since the fixture started.
    ///
    /// A window is abandoned because its placement was refused while the display was there, and it
    /// is retried because the display left and came back. Those are different facts, and only the
    /// second may let a placement succeed, so the fixture records the departure rather than
    /// inferring it from the display being present now.
    /// Track the wanted display across the fixture's life, so a driver refusal can tell "not
    /// attached yet, because the topology has not settled" from "attached once and then unplugged".
    #[derive(PartialEq, Eq)]
    enum WantedDisplayDeparture {
        NeverSeen,
        Attached,
        Departed,
    }

    /// Record the wanted display leaving, reading the topology the monitor plugin publishes.
    fn observe_wanted_display_departure(
        observation: Res<DisplayTopologyObservation>,
        mut fixture: ResMut<PlacementRetryFixture>,
    ) {
        let DisplayTopologyObservation::Observed(displays) = observation.as_ref() else {
            return;
        };
        if displays.is_empty() {
            return;
        }
        let Ok(scheme) = reporter::edid_serial_scheme() else {
            return;
        };
        let wanted = fixture.wanted.clone();
        let present = displays.iter().any(|display| {
            matches!(
                platform::classify_display_key(&display.device_evidence, &scheme),
                DisplayKeyClassification::Keyed(arrived) if arrived == wanted
            )
        });
        fixture.departure = match (&fixture.departure, present) {
            (WantedDisplayDeparture::NeverSeen, false) => WantedDisplayDeparture::NeverSeen,
            (WantedDisplayDeparture::NeverSeen | WantedDisplayDeparture::Attached, true) => {
                WantedDisplayDeparture::Attached
            },
            (WantedDisplayDeparture::Attached, false) | (WantedDisplayDeparture::Departed, _) => {
                WantedDisplayDeparture::Departed
            },
        };
    }

    /// Report the monitor entity the monitor topology spawned for the wanted display.
    ///
    /// Geometry alone cannot separate "placed back on the display it was saved to" from "fitted
    /// onto whatever was available", because a fallback fit can land on the same numbers; the
    /// monitor entity can.
    pub(crate) fn wanted_display_monitor(app: &App) -> Result<Entity, String> {
        let wanted = app
            .world()
            .resource::<PlacementRetryFixture>()
            .wanted
            .clone();
        let DisplayTopologyObservation::Observed(displays) =
            app.world().resource::<DisplayTopologyObservation>()
        else {
            return Err(String::from(
                "the monitor topology has not been observed yet",
            ));
        };
        let scheme = reporter::edid_serial_scheme()
            .map_err(|error| format!("failed to read the EDID serial scheme: {error}"))?;
        displays
            .iter()
            .find(|display| {
                matches!(
                    platform::classify_display_key(&display.device_evidence, &scheme),
                    DisplayKeyClassification::Keyed(arrived) if arrived == wanted
                )
            })
            .map(|display| display.entity)
            .ok_or_else(|| String::from("the wanted display is not in the current topology"))
    }

    /// A window driver whose apply outcome can change during a test.
    ///
    /// [`BoundDisplayWindowDriver`] fixes its outcome at construction, which is right for the
    /// tests that assert one steady state. A retry test needs an outcome that changes: the driver
    /// refuses while the saved display is gone and succeeds once it is back, so a re-attempted
    /// placement can actually land instead of failing a second time for the original reason.
    ///
    /// Success delegates to the production [`WindowEndpointDriver`] rather than reporting success
    /// on its own, because the saved geometry is written by the restore pipeline that driver
    /// starts. A driver that merely finished `Succeeded` would establish the role while the window
    /// still sat at its opening position, which is the state the retry exists to end.
    struct RetryWindowDriver(WindowEndpointDriver);

    impl EndpointDriver for RetryWindowDriver {
        type Configuration = EstablishedWindowPlacement;
        type Target = WindowPlacementTarget;

        fn resolve_target(
            &mut self,
            world: &mut World,
            context: &TargetResolutionContext<'_>,
            configuration: &Self::Configuration,
        ) -> TargetResolution<Self::Target> {
            self.0.resolve_target(world, context, configuration)
        }

        fn start_apply(
            &mut self,
            world: &mut World,
            context: ApplyContext<'_, Self::Configuration>,
            configuration: &Self::Configuration,
            target: Self::Target,
        ) {
            if wanted_display_has_returned(world) {
                self.0.start_apply(world, context, configuration, target);
                return;
            }
            context.into_completion().finish(DriverCompletion::Failed(
                DeviceAccessError::Unsupported {
                    detail: String::from("the saved display is not attached right now"),
                },
            ));
        }

        fn established(
            &mut self,
            world: &mut World,
            context: EstablishedContext<'_, Self::Configuration>,
        ) {
            self.0.established(world, context);
        }

        fn cancel_apply(
            &mut self,
            world: &mut World,
            role: &RoleKey,
            role_entity: DriverCleanupRoleEntity,
            attempt: AttemptRef,
            invalidation: AttemptInvalidation,
        ) {
            self.0
                .cancel_apply(world, role, role_entity, attempt, invalidation);
        }

        fn release_session(
            &mut self,
            world: &mut World,
            role: &RoleKey,
            role_entity: DriverCleanupRoleEntity,
            session: SessionRef,
            cause: SessionReleaseCause,
        ) {
            self.0
                .release_session(world, role, role_entity, session, cause);
        }
    }

    /// Whether the display the retry fixture's window is saved against is attached right now.
    ///
    /// The reporter's own classifier answers, against the same topology observation the retry
    /// observer reads, so the driver refuses and succeeds for the one reason the test is about:
    /// the display is gone, or the display came back. No harness flag can move it.
    fn wanted_display_has_returned(world: &World) -> bool {
        let Some(fixture) = world.get_resource::<PlacementRetryFixture>() else {
            return false;
        };
        if fixture.departure != WantedDisplayDeparture::Departed {
            return false;
        }
        // A real driver refusing an unattached display leaves the role stopped, and the kernel does
        // not revive a stopped role on its own — only a deliberate restart does. Keep refusing
        // while a window is still holding the abandoned placement, so an arrival that the observer
        // declined to match cannot be rescued by a retry the kernel would never have made.
        if world
            .iter_entities()
            .any(|entity| entity.contains::<PlacementAbandoned>())
        {
            return false;
        }
        let Some(DisplayTopologyObservation::Observed(displays)) =
            world.get_resource::<DisplayTopologyObservation>()
        else {
            return false;
        };
        let wanted = fixture.wanted.clone();
        let Ok(scheme) = reporter::edid_serial_scheme() else {
            return false;
        };
        displays.iter().any(|display| {
            matches!(
                platform::classify_display_key(&display.device_evidence, &scheme),
                DisplayKeyClassification::Keyed(arrived) if arrived == wanted
            )
        })
    }

    /// The saved record a retry script starts from.
    ///
    /// `NoUsableKey` seeds a retained v4 record with no durable evidence, which is the real shape
    /// of a pre-v5 file that never carried a display identity. That record reaches
    /// [`WantedDisplay::Unnamed`] through the persisted branch of
    /// [`WantedDisplay::for_role`], so it stays unnamed whether or not a binding exists.
    fn retry_window_state(
        script: &AbandonedPlacementScript,
        wanted_device: DeviceKey,
    ) -> PersistedWindowState {
        let mut window_state = saved_window_state(wanted_device);
        if matches!(script, AbandonedPlacementScript::NoUsableKey) {
            window_state.target = PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedDisplayIdentityV4::Anonymous,
            );
        }
        window_state
    }

    /// The display the fixture window sits on, which never leaves.
    ///
    /// `bevy_window::HasWindows` is a `linked_spawn` relationship target, so a window whose
    /// `OnMonitor` names a departing display's monitor is despawned with it. The cases here script
    /// the wanted display leaving, and a window that stops existing proves nothing about placement.
    const RETRY_HOST_DISPLAY: &str = "placement retry host display";
    const RETRY_WANTED_DISPLAY: &str = "placement retry wanted display";
    const RETRY_UNRELATED_DISPLAY: &str = "placement retry unrelated display";
    const RETRY_LOOKALIKE_DISPLAY: &str = "placement retry lookalike display";

    /// Script the three displays a retry test can see, with the wanted one already departed.
    ///
    /// The lookalike publishes no identity material at all, so the reporter names no key for it.
    /// That is the one arrival a resemblance test would wrongly accept, and the adapter is the only
    /// place its evidence can come from.
    fn retry_display_adapter() -> Result<(DisplayTestAdapter, DeviceKey), String> {
        let adapter = DisplayTestAdapter::new();
        adapter.observe_displays(vec![
            DisplayTestDescriptor::new(
                RETRY_HOST_DISPLAY,
                b"placement-retry-host".to_vec(),
                PlatformDeviceHandle::PlatformHasNoConcept,
            ),
            DisplayTestDescriptor::new(
                RETRY_WANTED_DISPLAY,
                b"placement-retry-wanted".to_vec(),
                PlatformDeviceHandle::PlatformHasNoConcept,
            ),
            DisplayTestDescriptor::new(
                RETRY_UNRELATED_DISPLAY,
                b"placement-retry-unrelated".to_vec(),
                PlatformDeviceHandle::PlatformHasNoConcept,
            ),
            DisplayTestDescriptor::without_published_identity(
                RETRY_LOOKALIKE_DISPLAY,
                PlatformDeviceHandle::PlatformHasNoConcept,
            ),
        ]);
        if let DisplayTestEnumeration::UnscriptedDisplay(name) =
            adapter.enumerate_displays(&[RETRY_HOST_DISPLAY, RETRY_WANTED_DISPLAY])
        {
            return Err(format!(
                "the retry fixture never scripted a display named {name}"
            ));
        }
        let DisplayTestDeviceKey::Keyed(wanted) = adapter.display_device_key(RETRY_WANTED_DISPLAY)
        else {
            return Err(String::from(
                "the display test adapter named no key for the wanted display",
            ));
        };
        Ok((adapter, wanted))
    }

    /// A saved primary window the placement deadline has already given up on.
    ///
    /// Returns once the [`ProjectedRolePlacementStatus::PlacementImpossible`] arm has run, so the
    /// window already carries [`WindowRevealDisposition::PlacementAbandoned`] and
    /// [`PlacementAbandoned`]. The returned `Entity` is the primary window.
    ///
    /// The app is the production plugin set: `WindowManagerPlugin` brings the real monitor
    /// topology, so display arrivals are enumerated by the adapter and `update_monitors` publishes
    /// them and fires [`MonitorConnected`] on its own. Nothing here writes `OnMonitor` or the
    /// topology by hand.
    ///
    /// The binding takes [`RetryWindowDriver`] rather than the plugin's own driver because a
    /// departed display alone does not reach the abandoned state: with the device still reported,
    /// the role sits in `Applying` forever. A driver that refuses while the display is gone is what
    /// stops the role, and a stopped role is exactly the case the retry makes recoverable.
    pub(crate) fn abandoned_placement_app(
        script: AbandonedPlacementScript,
    ) -> Result<(App, Entity), String> {
        let (adapter, wanted) = retry_display_adapter()?;
        let window_state = tempdir()
            .map_err(|error| format!("failed to create the window state directory: {error}"))?;
        let mut app = App::new();
        // Pin the platform before the plugin builds, so this fixture asserts the same restore
        // branches on every host. `Platform::detect` reads the environment: on a headless Linux
        // runner it reports `X11`, whose windowed restore waits forever for the
        // `_NET_FRAME_EXTENTS` reply that gates `X11FrameCompensated`, and the returned display
        // never places the window. The plugin harness in `lib.rs` pins the same way.
        app.insert_resource(Platform::MacOs);
        app.add_plugins((
            MinimalPlugins,
            WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            },
            adapter.clone(),
            WindowManagerPlugin::with_path(window_state.path().join("windows.ron")),
        ))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
        .add_systems(
            Update,
            (
                observe_wanted_display_departure,
                report_window_on_its_targeted_monitor,
            ),
        );
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .set_max_delta(Duration::from_secs_f32(EXACT_DISPLAY_WAIT_TIMEOUT_SECS));

        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create the primary role: {error}"))?;
        app.world_mut()
            .resource_mut::<PersistedWindowPlacements>()
            .seed(HashMap::from([(
                role.clone(),
                retry_window_state(&script, wanted.clone()),
            )]));
        app.world_mut().run_schedule(PreStartup);
        let driver = app.add_endpoint_driver(RetryWindowDriver(WindowEndpointDriver));
        // Production window binding authoring reaches for `WindowDriverId`, so a role it authors
        // for this window would run the real driver and place the window the fixture is holding
        // unplaced. Point that registration at the refusing driver too.
        app.insert_resource(WindowDriverId(driver));
        let role_entity = register_binding(
            app.world_mut(),
            retry_binding_authoring(role, wanted.clone(), driver),
        )
        .map_err(|error| format!("failed to register the retry window binding: {error}"))?;
        app.insert_resource(PlacementRetryFixture {
            adapter,
            wanted,
            departure: WantedDisplayDeparture::NeverSeen,
            window_state,
        });
        for _ in 0..4 {
            app.update();
        }

        let monitor = retry_host_monitor(&mut app)?;
        let window = spawn_production_fallback_window(&mut app, role_entity, monitor);
        app.world_mut()
            .entity_mut(window)
            .insert(SavedDisplayRevealWait::default());
        app.world_mut().init_resource::<InjectedWinitWindows>();
        app.world_mut()
            .resource_mut::<InjectedWinitWindows>()
            .insert(window, UVec2::ZERO);
        for _ in 0..8 {
            app.update();
        }
        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);
        if app.world().get::<PlacementAbandoned>(window).is_none() {
            return Err(String::from(
                "the placement deadline did not abandon the retry fixture window",
            ));
        }
        Ok((app, window))
    }

    /// Stand in for winit reporting a window's display after the operating system honours a move.
    ///
    /// `OnMonitor` is written by winit alone, and `CurrentMonitorEntity` follows it, so a fixture
    /// with no winit leaves a moved window associated with the display it started on however
    /// correctly the restore placed it. The association is taken from the monitor the restore
    /// preparation itself targeted, which is production's own decision rather than the test's;
    /// deriving it from geometry is not open to the fixture, because every scripted monitor is
    /// published at `IVec2::ZERO` with a one-pixel size.
    fn report_window_on_its_targeted_monitor(
        windows: Query<(Entity, &WindowRestoreAttempt), With<ManagedWindow>>,
        driver_state: Res<WindowRoleDriverState>,
        mut commands: Commands,
    ) {
        for (window, restore_attempt) in &windows {
            let RestoreRecord::UnderPreparation(preparation) =
                driver_state.restore_record(restore_attempt.attempt())
            else {
                continue;
            };
            commands
                .entity(window)
                .insert(OnMonitor(preparation.target().monitor));
        }
    }

    fn retry_binding_authoring(
        role: RoleKey,
        device: DeviceKey,
        driver: EndpointDriverRegistration<EstablishedWindowPlacement>,
    ) -> BindingAuthoring<EstablishedWindowPlacement> {
        BindingAuthoring::new(
            role,
            DeviceEndpoint {
                device,
                id: EndpointId::Whole,
            },
            driver,
            saved_placement(),
            BindingPolicy::new(
                RecoveryPolicy::ReapplyOnReturn,
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        )
    }

    /// Report one display arriving, by enumerating it on the display platform.
    ///
    /// The adapter is the platform, so this is the whole of an arrival: `update_monitors` sees the
    /// new enumeration, publishes the topology, spawns the monitor entity and fires
    /// [`MonitorConnected`] itself. Nothing here writes the topology, the monitor set or a window's
    /// monitor association, which is what keeps the retry tests a test of the observer rather than
    /// of the fixture.
    ///
    /// The unrelated display stays enumerated throughout, so every case has a display present and
    /// the cases differ only in which one arrives.
    pub(crate) fn script_display_arrival(
        app: &mut App,
        arrival: RetryDisplayArrival,
    ) -> Result<(), String> {
        match arrival {
            RetryDisplayArrival::WantedDisplay => {
                enumerate_retry_displays(app, &[RETRY_HOST_DISPLAY])?;
                enumerate_retry_displays(app, &[RETRY_HOST_DISPLAY, RETRY_WANTED_DISPLAY])?;
            },
            RetryDisplayArrival::UnrelatedDisplay => {
                enumerate_retry_displays(
                    app,
                    &[
                        RETRY_HOST_DISPLAY,
                        RETRY_WANTED_DISPLAY,
                        RETRY_UNRELATED_DISPLAY,
                    ],
                )?;
            },
            RetryDisplayArrival::EvidenceOnlyLookalike => {
                enumerate_retry_displays(
                    app,
                    &[
                        RETRY_HOST_DISPLAY,
                        RETRY_WANTED_DISPLAY,
                        RETRY_LOOKALIKE_DISPLAY,
                    ],
                )?;
            },
        }
        Ok(())
    }

    /// Point the display platform at exactly these displays and let the topology settle.
    fn enumerate_retry_displays(app: &mut App, displays: &[&str]) -> Result<(), String> {
        let enumeration = app
            .world()
            .resource::<PlacementRetryFixture>()
            .adapter
            .enumerate_displays(displays);
        if let DisplayTestEnumeration::UnscriptedDisplay(name) = enumeration {
            return Err(format!(
                "the retry fixture never scripted a display named {name}"
            ));
        }
        for _ in 0..16 {
            app.update();
        }
        Ok(())
    }

    pub(crate) fn bound_window_app(
        bound_display: BoundDisplay,
        bound_window: BoundWindow,
        configuration_history: BoundConfigurationHistory,
        recovery: RecoveryPolicy,
    ) -> Result<(App, Entity), String> {
        let apply_progress = match configuration_history {
            BoundConfigurationHistory::SavedDuringSession => BoundApplyProgress::Succeeded,
            BoundConfigurationHistory::LoadedAtStartup | BoundConfigurationHistory::NeverSaved => {
                BoundApplyProgress::Pending
            },
        };
        bound_window_app_with_apply_progress(
            bound_display,
            bound_window,
            configuration_history,
            apply_progress,
            recovery,
        )
    }

    fn bound_window_app_with_apply_progress(
        bound_display: BoundDisplay,
        bound_window: BoundWindow,
        configuration_history: BoundConfigurationHistory,
        apply_progress: BoundApplyProgress,
        recovery: RecoveryPolicy,
    ) -> Result<(App, Entity), String> {
        let mut app = App::new();
        output_proof::register_window_output_proof(&mut app);
        app.register_rigging_role_relationship::<WindowRiggingRole>();
        app.add_plugins(MinimalPlugins)
            .add_plugins(RiggingPlugin)
            .init_resource::<PersistedWindowPlacements>()
            .init_resource::<WindowFallbackRecoveryState>()
            .init_resource::<StrandedWindowMovementBaselines>()
            .init_resource::<ManagedWindowRegistry>()
            .init_resource::<EndedAttemptEvents>()
            .insert_resource(Platform::FIXTURE)
            .add_observer(count_ended_attempt)
            .add_observer(managed::on_managed_window_added)
            .add_observer(crate::mark_primary_window_as_managed)
            .add_systems(
                Update,
                (
                    abandon_placement_after_deadline,
                    show_window_once_placement_settles,
                    managed::adopt_live_display_for_stranded_window,
                )
                    .chain(),
            );
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .set_max_delta(Duration::from_secs_f32(EXACT_DISPLAY_WAIT_TIMEOUT_SECS));
        let role = bound_window.role()?;
        let (scheme_name, saved_device, live_device) = test_display_devices()?;
        app.register_device_scheme(scheme_name);
        let monitor = app.world_mut().spawn_empty().id();
        let report_state = Arc::new(Mutex::new(bound_display));
        let reporter = app.add_device_reporter(
            BoundDisplayReporter {
                state: Arc::clone(&report_state),
                saved_device: saved_device.clone(),
                live_device,
                monitor,
            },
            bound_display_reporter_registration(bound_display),
        );
        add_present_reference_reporter(&mut app, bound_display, &saved_device, monitor);
        let driver = app.add_endpoint_driver(BoundDisplayWindowDriver {
            bound_display,
            apply_progress,
            reporter,
        });
        app.insert_resource(WindowDriverId(driver));
        app.insert_resource(BoundDisplayReportState(report_state))
            .insert_resource(BoundDisplayReporterId(reporter));
        if matches!(
            configuration_history,
            BoundConfigurationHistory::LoadedAtStartup
        ) {
            app.world_mut()
                .resource_mut::<PersistedWindowPlacements>()
                .seed(HashMap::from([(
                    role.clone(),
                    saved_window_state(saved_device.clone()),
                )]));
        }
        let binding = BindingAuthoring::new(
            role,
            DeviceEndpoint {
                device: saved_device,
                id:     EndpointId::Whole,
            },
            driver,
            saved_placement(),
            BindingPolicy::new(
                recovery,
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        );
        let role_entity = register_binding(app.world_mut(), binding)
            .map_err(|error| format!("failed to register the window binding: {error}"))?;
        let window = bound_window.spawn(app.world_mut(), monitor);
        app.world_mut().entity_mut(window).insert((
            WindowBindingAuthoring::Registered,
            managed::WindowRiggingRole::new(role_entity),
        ));
        settle_bound_window_app(&mut app, configuration_history);
        disable_target_reporter(&mut app, bound_display, reporter)?;
        Ok((app, window))
    }

    fn add_present_reference_reporter(
        app: &mut App,
        bound_display: BoundDisplay,
        saved_device: &DeviceKey,
        monitor: Entity,
    ) {
        if matches!(bound_display, BoundDisplay::ReporterDisabled) {
            app.add_device_reporter(
                PresentDisplayReporter {
                    device: saved_device.clone(),
                    monitor,
                },
                ReporterRegistration::required(
                    DiscoveryCadence::OnDemand,
                    ReporterCoverage::MatchingEvidenceOnly,
                    Duration::from_secs(10),
                ),
            );
        }
    }

    fn disable_target_reporter(
        app: &mut App,
        bound_display: BoundDisplay,
        reporter: ReporterId,
    ) -> Result<(), String> {
        if matches!(bound_display, BoundDisplay::ReporterDisabled) {
            app.world_mut()
                .resource_mut::<DiscoveryControl>()
                .disable(reporter)
                .map_err(|error| format!("failed to disable the target reporter: {error}"))?;
            app.update();
        }
        Ok(())
    }

    fn bound_display_reporter_registration(bound_display: BoundDisplay) -> ReporterRegistration {
        match bound_display {
            BoundDisplay::ReporterDisabled => ReporterRegistration::optional(
                DiscoveryCadence::OnDemand,
                ReporterActivation::Enabled,
                ReporterCoverage::MatchingEvidenceOnly,
                Duration::from_secs(10),
            ),
            BoundDisplay::Live | BoundDisplay::Absent => ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::EstablishesAbsence(AuthoritativeReporterCoverage::one(
                    CoveredDeviceIdentitySpace::AllKeysOfKind {
                        kind: DeviceKind::Display,
                    },
                )),
                Duration::from_secs(10),
            ),
        }
    }

    fn observe_fallback_baseline(app: &mut App, role: &RoleKey) -> Result<(), String> {
        let endpoint = app
            .world()
            .resource::<Bindings>()
            .binding(role)
            .map_err(|error| format!("the primary binding disappeared: {error}"))?
            .endpoint
            .clone();
        let driver = app.world().resource::<WindowDriverId>().0;
        replace_binding(
            app.world_mut(),
            BindingAuthoring::new(
                role.clone(),
                endpoint,
                driver,
                saved_placement(),
                BindingPolicy::new(
                    RecoveryPolicy::ReapplyOnReturn,
                    RetryOn::NewRevision,
                    OnAbort::default(),
                    OnSessionLoss::default(),
                    ApplyDeadline::ProcessDefault,
                ),
            ),
        )
        .map_err(|error| format!("failed to return the primary binding to waiting: {error}"))?;
        app.world_mut()
            .run_system_once(managed::adopt_live_display_for_stranded_window)
            .map_err(|error| format!("failed to record the fallback baseline: {error}"))
    }

    /// An application whose primary role owns a previously established configuration.
    fn bound_primary_window_app(bound_display: BoundDisplay) -> Result<App, String> {
        bound_window_app(
            bound_display,
            BoundWindow::Primary,
            BoundConfigurationHistory::LoadedAtStartup,
            RecoveryPolicy::ReapplyOnReturn,
        )
        .map(|(app, _)| app)
    }

    fn install_active_only_persistence_writer(app: &mut App) -> Result<TempDir, String> {
        let directory = tempdir()
            .map_err(|error| format!("failed to create persistence directory: {error}"))?;
        app.insert_resource(RestoreWindowConfig {
            path: directory.path().join("windows.ron"),
        })
        .insert_resource(ManagedWindowPersistence::ActiveOnly)
        .init_resource::<ManagedWindowRegistry>()
        .add_systems(
            Update,
            persistence::write_established_window_configurations_for_test
                .before(abandon_placement_after_deadline),
        );
        Ok(directory)
    }

    #[test]
    fn saved_geometry_disposition_reveals_a_normal_restore_window() -> Result<(), String> {
        let (mut app, primary_window) = bound_window_app(
            BoundDisplay::Live,
            BoundWindow::Primary,
            BoundConfigurationHistory::LoadedAtStartup,
            RecoveryPolicy::ReapplyOnReturn,
        )?;
        app.world_mut()
            .entity_mut(primary_window)
            .insert(WindowRevealDisposition::SavedGeometryApplied);

        app.update();

        assert!(window_is_visible(&mut app));
        assert_eq!(
            recovery_phase(&app),
            WindowFallbackRecoveryProgress::NotRecovering
        );
        Ok(())
    }

    #[test]
    fn live_saved_display_keeps_waiting_for_normal_placement() -> Result<(), String> {
        let (mut app, primary_window) = bound_window_app(
            BoundDisplay::Live,
            BoundWindow::Primary,
            BoundConfigurationHistory::LoadedAtStartup,
            RecoveryPolicy::ReapplyOnReturn,
        )?;

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        assert!(!entity_is_visible(&app, primary_window));
        assert!(has_reveal_wait(&app, primary_window));
        assert!(
            app.world()
                .get::<WindowRevealDisposition>(primary_window)
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn stopped_role_with_live_display_reveals_at_deadline() -> Result<(), String> {
        let (mut app, primary_window) = bound_window_app_with_apply_progress(
            BoundDisplay::Live,
            BoundWindow::Primary,
            BoundConfigurationHistory::LoadedAtStartup,
            BoundApplyProgress::Unsupported,
            RecoveryPolicy::ReapplyOnReturn,
        )?;
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create the primary role: {error}"))?;
        let binding_entity = app
            .world()
            .resource::<Bindings>()
            .role_entity(&role)
            .map_err(|_| String::from("the primary role was not projected"))?;
        let status = app
            .world()
            .get::<RoleStatus>(binding_entity)
            .ok_or_else(|| String::from("the primary role has no projected status"))?;
        assert!(matches!(
            status.view(),
            RoleStatusView::Stopped(StoppedStatusView::Unsupported { .. })
        ));

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        assert!(entity_is_visible(&app, primary_window));
        assert!(!has_reveal_wait(&app, primary_window));
        assert_eq!(
            app.world().get::<WindowRevealDisposition>(primary_window),
            Some(&WindowRevealDisposition::PlacementAbandoned)
        );
        Ok(())
    }

    #[test]
    fn departure_grace_wait_reveals_at_deadline() -> Result<(), String> {
        let (mut app, primary_window) = bound_window_app_with_apply_progress(
            BoundDisplay::Live,
            BoundWindow::Primary,
            BoundConfigurationHistory::SavedDuringSession,
            BoundApplyProgress::Succeeded,
            RecoveryPolicy::ReapplyOnRequest,
        )?;
        report_bound_display(&mut app, BoundDisplay::Absent)?;
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create the primary role: {error}"))?;
        let binding_entity = app
            .world()
            .resource::<Bindings>()
            .role_entity(&role)
            .map_err(|_| String::from("the primary role was not projected"))?;
        let status = app
            .world()
            .get::<RoleStatus>(binding_entity)
            .ok_or_else(|| String::from("the primary role has no projected status"))?;
        assert!(matches!(
            status.view(),
            RoleStatusView::Waiting(WaitingStatusView::Reporter(
                HardwareWait::DepartureGrace { .. }
            ))
        ));
        let Some(mut window) = app.world_mut().get_mut::<Window>(primary_window) else {
            return Err(String::from("the primary window disappeared"));
        };
        window.visible = false;
        app.world_mut()
            .entity_mut(primary_window)
            .insert(SavedDisplayRevealWait::default())
            .remove::<WindowRevealDisposition>();

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        assert!(entity_is_visible(&app, primary_window));
        assert_eq!(
            app.world().get::<WindowRevealDisposition>(primary_window),
            Some(&WindowRevealDisposition::FittedToFallbackDisplay)
        );
        assert!(!has_reveal_wait(&app, primary_window));
        Ok(())
    }

    #[derive(Clone, Copy)]
    enum ProductionReporterWaitCase {
        AwaitingFirstReport,
        Unconfirmed,
        Unreachable,
    }

    impl ProductionReporterWaitCase {
        const fn expected_crossing(self) -> ReporterBoundCrossing {
            match self {
                Self::AwaitingFirstReport => ReporterBoundCrossing::AwaitingFirstReport,
                Self::Unconfirmed => ReporterBoundCrossing::Unconfirmed,
                Self::Unreachable => ReporterBoundCrossing::Unreachable,
            }
        }

        fn pre_failure_status_reached(self, role_status: &RoleStatus) -> bool {
            match (self, role_status.view()) {
                (
                    Self::AwaitingFirstReport,
                    RoleStatusView::Waiting(WaitingStatusView::Reporter(
                        HardwareWait::AwaitingFirstReport {
                            timing: WaitTiming::Bounded { .. },
                            ..
                        },
                    )),
                )
                | (
                    Self::Unconfirmed,
                    RoleStatusView::Waiting(WaitingStatusView::Reporter(
                        HardwareWait::Unconfirmed {
                            timing: WaitTiming::Bounded { .. },
                            ..
                        },
                    )),
                )
                | (
                    Self::Unreachable,
                    RoleStatusView::Waiting(WaitingStatusView::Reporter(
                        HardwareWait::Unreachable {
                            timing: WaitTiming::Bounded { .. },
                            ..
                        },
                    )),
                ) => true,
                (
                    Self::AwaitingFirstReport | Self::Unconfirmed | Self::Unreachable,
                    RoleStatusView::Applying { .. }
                    | RoleStatusView::Waiting(_)
                    | RoleStatusView::Established { .. }
                    | RoleStatusView::Stopped(_),
                ) => false,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum WaitTransition {
        BoundCrossed(ReporterBoundCrossing),
        OtherStatusChange,
    }

    #[derive(Default, Resource)]
    struct ObservedWaitTransitions(Vec<WaitTransition>);

    fn record_reporter_bound_crossing(
        event: On<LiveRoleChanged>,
        role_statuses: Query<&RoleStatus>,
        mut observed: ResMut<ObservedWaitTransitions>,
    ) {
        let Ok(role_status) = role_statuses.get(event.binding) else {
            return;
        };
        let transition = match role_status.view() {
            RoleStatusView::Waiting(WaitingStatusView::Reporter(
                HardwareWait::AwaitingFirstReport {
                    timing: WaitTiming::Overdue { .. },
                    ..
                },
            )) => WaitTransition::BoundCrossed(ReporterBoundCrossing::AwaitingFirstReport),
            RoleStatusView::Waiting(WaitingStatusView::Reporter(HardwareWait::Unconfirmed {
                timing: WaitTiming::Overdue { .. },
                ..
            })) => WaitTransition::BoundCrossed(ReporterBoundCrossing::Unconfirmed),
            RoleStatusView::Waiting(WaitingStatusView::Reporter(HardwareWait::Unreachable {
                timing: WaitTiming::Overdue { .. },
                ..
            })) => WaitTransition::BoundCrossed(ReporterBoundCrossing::Unreachable),
            RoleStatusView::Applying { .. }
            | RoleStatusView::Waiting(_)
            | RoleStatusView::Established { .. }
            | RoleStatusView::Stopped(_) => WaitTransition::OtherStatusChange,
        };
        if matches!(transition, WaitTransition::BoundCrossed(_)) {
            observed.0.push(transition);
        }
    }

    fn display_is_unreachable(app: &mut App, saved_device: &DeviceKey) -> bool {
        let world = app.world_mut();
        let mut devices = world.query::<(&DeviceKey, &Presence)>();
        devices.iter(world).any(|(device, presence)| {
            device == saved_device && matches!(presence, Presence::Unreachable { .. })
        })
    }

    pub(crate) fn production_window_status(
        app: &App,
        window: Entity,
    ) -> Result<&RoleStatus, String> {
        let role = app
            .world()
            .get::<WindowRiggingRole>(window)
            .ok_or_else(|| String::from("the production window has no rigging role"))?;
        app.world()
            .get::<RoleStatus>(role.entity())
            .ok_or_else(|| String::from("the production window role has no projected status"))
    }

    fn register_production_window_binding(
        app: &mut App,
        role: RoleKey,
        saved_device: DeviceKey,
    ) -> Result<Entity, String> {
        let driver = app.world().resource::<WindowDriverId>().0;
        let binding = BindingAuthoring::new(
            role,
            DeviceEndpoint {
                device: saved_device,
                id:     EndpointId::Whole,
            },
            driver,
            saved_placement(),
            BindingPolicy::new(
                RecoveryPolicy::Forget,
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        );
        register_binding(app.world_mut(), binding)
            .map_err(|error| format!("failed to register the production window binding: {error}"))
    }

    fn production_reporter_test_adapter(
        reporter_wait_case: ProductionReporterWaitCase,
    ) -> Result<(DisplayTestAdapter, DeviceKey), String> {
        const DISPLAY_EVIDENCE: &[u8] = b"fallback-reveal-saved-display";
        const DISPLAY_NAME: &str = "fallback reveal saved display";
        const SECOND_DISPLAY_NAME: &str = "fallback reveal duplicate display";

        let display_test_adapter = DisplayTestAdapter::new();
        let mut displays = vec![DisplayTestDescriptor::new(
            DISPLAY_NAME,
            DISPLAY_EVIDENCE,
            PlatformDeviceHandle::PlatformHasNoConcept,
        )];
        if matches!(reporter_wait_case, ProductionReporterWaitCase::Unconfirmed) {
            displays.push(DisplayTestDescriptor::new(
                SECOND_DISPLAY_NAME,
                DISPLAY_EVIDENCE,
                PlatformDeviceHandle::PlatformHasNoConcept,
            ));
        }
        display_test_adapter.observe_displays(displays);
        let DisplayTestDeviceKey::Keyed(saved_device) =
            display_test_adapter.display_device_key(DISPLAY_NAME)
        else {
            return Err(String::from(
                "the display test adapter did not retain the saved display",
            ));
        };
        if matches!(
            reporter_wait_case,
            ProductionReporterWaitCase::AwaitingFirstReport
        ) {
            display_test_adapter.fail_reporter_enumeration();
        }
        Ok((display_test_adapter, saved_device))
    }

    fn production_window_manager_app(
        display_test_adapter: &DisplayTestAdapter,
        saved_device: &DeviceKey,
    ) -> Result<(App, Entity, TempDir), String> {
        let window_state = tempdir()
            .map_err(|error| format!("failed to create the window state directory: {error}"))?;
        let mut app = App::new();
        // Pinned before the plugin builds; `configured_platform` reads it instead of the
        // host session.
        app.insert_resource(Platform::FIXTURE);
        app.add_plugins((
            MinimalPlugins,
            WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            },
            display_test_adapter.clone(),
            WindowManagerPlugin::with_path(window_state.path().join("windows.ron")),
        ))
        .init_resource::<ObservedWaitTransitions>()
        .add_observer(record_reporter_bound_crossing)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .set_max_delta(Duration::from_secs(1));

        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create the primary role: {error}"))?;
        app.world_mut()
            .resource_mut::<PersistedWindowPlacements>()
            .seed(HashMap::from([(
                role.clone(),
                saved_window_state(saved_device.clone()),
            )]));
        app.world_mut().run_schedule(PreStartup);
        let role_entity = register_production_window_binding(&mut app, role, saved_device.clone())?;
        Ok((app, role_entity, window_state))
    }

    fn drive_reporter_to_pre_failure_setup(
        app: &mut App,
        display_test_adapter: &DisplayTestAdapter,
        saved_device: &DeviceKey,
        reporter_wait_case: ProductionReporterWaitCase,
    ) -> Result<(), String> {
        if matches!(reporter_wait_case, ProductionReporterWaitCase::Unreachable) {
            for _ in 0..16 {
                app.update();
            }
            if !app.world().iter_entities().any(|entity| {
                entity.get::<DeviceKey>() == Some(saved_device)
                    && matches!(entity.get::<Presence>(), Some(Presence::Present))
            }) {
                return Err(String::from(
                    "the production display reporter did not publish the saved display",
                ));
            }
            display_test_adapter.fail_reporter_enumeration();
            app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs(1)));
            for _ in 0..64 {
                app.update();
                if display_is_unreachable(app, saved_device) {
                    break;
                }
            }
            if !display_is_unreachable(app, saved_device) {
                return Err(String::from(
                    "the production display reporter did not publish an unreachable device",
                ));
            }
            app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
        } else {
            for _ in 0..16 {
                app.update();
            }
        }
        Ok(())
    }

    /// The monitor entity standing for [`RETRY_HOST_DISPLAY`].
    fn retry_host_monitor(app: &mut App) -> Result<Entity, String> {
        let world = app.world_mut();
        let mut monitors = world.query::<(Entity, &Monitor)>();
        monitors
            .iter(world)
            .find(|(_, monitor)| monitor.name.as_deref() == Some(RETRY_HOST_DISPLAY))
            .map(|(entity, _)| entity)
            .ok_or_else(|| String::from("the display test adapter produced no host monitor"))
    }

    fn production_fallback_monitor(app: &mut App) -> Result<Entity, String> {
        let world = app.world_mut();
        let mut monitors = world.query_filtered::<Entity, With<Monitor>>();
        monitors
            .iter(world)
            .next()
            .ok_or_else(|| String::from("the display test adapter produced no monitor"))
    }

    fn spawn_production_fallback_window(
        app: &mut App,
        role_entity: Entity,
        fallback_monitor: Entity,
    ) -> Entity {
        let primary_window = app
            .world_mut()
            .spawn((
                Window {
                    visible: false,
                    ..default()
                },
                PrimaryWindow,
                CurrentMonitor {
                    descriptor:            MonitorDescriptor::for_current_enumeration(
                        0,
                        1.0,
                        IVec2::ZERO,
                        UVec2::ONE,
                    ),
                    effective_window_mode: WindowMode::Windowed,
                },
                CurrentMonitorEntity::new(fallback_monitor),
                OnMonitor(fallback_monitor),
                WindowBindingAuthoring::Registered,
                managed::WindowRiggingRole::new(role_entity),
            ))
            .id();
        app.world_mut()
            .entity_mut(primary_window)
            .remove::<SavedDisplayRevealWait>();
        primary_window
    }

    fn await_production_pre_failure_status(
        app: &mut App,
        primary_window: Entity,
        reporter_wait_case: ProductionReporterWaitCase,
    ) -> Result<(), String> {
        for _ in 0..64 {
            app.update();
            let Ok(role_status) = production_window_status(app, primary_window) else {
                continue;
            };
            if reporter_wait_case.pre_failure_status_reached(role_status) {
                break;
            }
        }
        let role_status = production_window_status(app, primary_window)?;
        if !reporter_wait_case.pre_failure_status_reached(role_status) {
            return Err(format!(
                "the production window did not reach its pre-failure state: {:?}",
                role_status.view()
            ));
        }
        Ok(())
    }

    fn production_reporter_wait_app(
        reporter_wait_case: ProductionReporterWaitCase,
    ) -> Result<(App, Entity, DisplayTestAdapter, TempDir), String> {
        let (display_test_adapter, saved_device) =
            production_reporter_test_adapter(reporter_wait_case)?;
        let (mut app, role_entity, window_state) =
            production_window_manager_app(&display_test_adapter, &saved_device)?;
        drive_reporter_to_pre_failure_setup(
            &mut app,
            &display_test_adapter,
            &saved_device,
            reporter_wait_case,
        )?;
        let fallback_monitor = production_fallback_monitor(&mut app)?;
        let primary_window =
            spawn_production_fallback_window(&mut app, role_entity, fallback_monitor);
        await_production_pre_failure_status(&mut app, primary_window, reporter_wait_case)?;
        if matches!(reporter_wait_case, ProductionReporterWaitCase::Unconfirmed) {
            display_test_adapter.fail_reporter_enumeration();
        }

        Ok((app, primary_window, display_test_adapter, window_state))
    }

    #[test]
    fn every_second_reporter_wait_reveals_after_each_supported_bound_crossing() -> Result<(), String>
    {
        const MAX_FAILURE_FRAMES: usize = 96;
        let cases = [
            ProductionReporterWaitCase::AwaitingFirstReport,
            ProductionReporterWaitCase::Unconfirmed,
            ProductionReporterWaitCase::Unreachable,
        ];

        for reporter_wait_case in cases {
            let expected_crossing = reporter_wait_case.expected_crossing();
            let (mut app, primary_window, display_test_adapter, window_state) =
                production_reporter_wait_app(reporter_wait_case)?;
            app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs(1)));
            for _ in 0..MAX_FAILURE_FRAMES {
                app.update();
                let crossed_once = app
                    .world()
                    .resource::<ObservedWaitTransitions>()
                    .0
                    .as_slice()
                    == [WaitTransition::BoundCrossed(expected_crossing)];
                if crossed_once {
                    break;
                }
            }

            assert_eq!(
                app.world().resource::<ObservedWaitTransitions>().0,
                [WaitTransition::BoundCrossed(expected_crossing)]
            );
            assert!(!entity_is_visible(&app, primary_window));
            assert!(!has_reveal_wait(&app, primary_window));
            app.world_mut()
                .entity_mut(primary_window)
                .insert(SavedDisplayRevealWait::default());
            let role = persistence::primary_window_role()
                .map_err(|error| format!("failed to recreate the primary role: {error}"))?;
            let configuration = app
                .world()
                .resource::<Bindings>()
                .configuration_for(&role)
                .map_err(|error| format!("the fallback role lost its configuration: {error}"))?;
            let (AvailableConfiguration::LastKnownGood(configuration)
            | AvailableConfiguration::Requested(configuration)) = configuration;
            let requested_placement = configuration
                .as_any()
                .downcast_ref::<EstablishedWindowPlacement>()
                .ok_or_else(|| {
                    String::from("the fallback role has the wrong configuration type")
                })?;
            assert_eq!(
                requested_placement.logical_size,
                UVec2::new(SAVED_LOGICAL_WIDTH, SAVED_LOGICAL_HEIGHT)
            );
            let fallback_monitor = app
                .world()
                .get::<CurrentMonitor>(primary_window)
                .ok_or_else(|| String::from("the adapter fallback monitor disappeared"))?;
            assert_eq!(fallback_monitor.descriptor.physical_size, UVec2::ONE);
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(Duration::from_secs_f32(EXACT_DISPLAY_WAIT_TIMEOUT_SECS));
            app.world_mut()
                .run_system_once(abandon_placement_after_deadline)
                .map_err(|error| format!("fallback placement failed: {error}"))?;
            assert_eq!(
                app.world().get::<WindowRevealDisposition>(primary_window),
                Some(&WindowRevealDisposition::FittedToFallbackDisplay)
            );
            app.world_mut()
                .run_system_once(show_window_once_placement_settles)
                .map_err(|error| format!("fallback reveal failed: {error}"))?;
            assert_adapter_fallback_window_placement(&app, primary_window)?;
            assert!(!has_reveal_wait(&app, primary_window));
            drop(display_test_adapter);
            drop(window_state);
        }

        Ok(())
    }

    #[test]
    fn disabled_reporter_allows_saved_window_fallback_at_the_deadline() -> Result<(), String> {
        let (mut app, primary_window) = bound_window_app(
            BoundDisplay::ReporterDisabled,
            BoundWindow::Primary,
            BoundConfigurationHistory::LoadedAtStartup,
            RecoveryPolicy::ReapplyOnReturn,
        )?;
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create the primary role: {error}"))?;
        let role_entity = app
            .world()
            .resource::<Bindings>()
            .role_entity(&role)
            .map_err(|_| String::from("the primary role was not projected"))?;
        assert!(matches!(
            app.world()
                .get::<RoleStatus>(role_entity)
                .map(RoleStatus::view),
            Some(RoleStatusView::Waiting(WaitingStatusView::Reporter(
                HardwareWait::Unconfirmed {
                    timing: WaitTiming::Unbounded { .. },
                    ..
                }
            )))
        ));

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        assert_fallback_window_placement(&app, primary_window)?;
        assert_eq!(
            app.world().get::<WindowRevealDisposition>(primary_window),
            Some(&WindowRevealDisposition::FittedToFallbackDisplay)
        );
        assert!(!has_reveal_wait(&app, primary_window));
        Ok(())
    }

    #[test]
    fn reveals_a_hidden_managed_window_bound_to_a_live_display_when_never_saved()
    -> Result<(), String> {
        let name = "never-saved-inspector";
        let (mut app, managed_window) = bound_window_app(
            BoundDisplay::Live,
            BoundWindow::Managed(name),
            BoundConfigurationHistory::NeverSaved,
            RecoveryPolicy::ReapplyOnReturn,
        )?;
        let role = persistence::managed_window_role(name)
            .map_err(|error| format!("failed to create managed role: {error}"))?;

        assert!(matches!(
            app.world()
                .resource::<PersistedWindowPlacements>()
                .get(&role),
            persistence::PersistedWindowPlacementLookup::NotSaved
        ));
        let binding = app
            .world()
            .resource::<Bindings>()
            .binding(&role)
            .map_err(|error| format!("managed binding disappeared: {error}"))?;
        assert!(matches!(
            &binding.last_known_good,
            LastKnownGoodConfiguration::NotEstablished
        ));

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        assert!(entity_is_visible(&app, managed_window));
        assert!(!has_reveal_wait(&app, managed_window));
        assert_eq!(
            recovery_phase_for_role(&app, &role),
            WindowFallbackRecoveryProgress::NotRecovering
        );
        assert!(
            !app.world()
                .resource::<StrandedWindowMovementBaselines>()
                .is_tracked(&role)
        );
        Ok(())
    }

    /// A binding whose display is unplugged is the stranded case: the kernel never resolves the
    /// endpoint, so no restore ever lifts the startup hide and the reveal has to.
    #[test]
    fn reveals_a_bound_role_whose_display_is_absent() -> Result<(), String> {
        let mut app = bound_primary_window_app(BoundDisplay::Absent)?;
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create the primary role: {error}"))?;

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        assert!(window_is_visible(&mut app));
        assert_eq!(
            recovery_phase(&app),
            WindowFallbackRecoveryProgress::Recovering(
                WindowFallbackRecoveryPhase::MissingLiveMonitor
            )
        );
        assert!(
            app.world()
                .resource::<StrandedWindowMovementBaselines>()
                .is_tracked(&role)
        );
        Ok(())
    }

    #[test]
    fn absent_saved_display_records_fallback_disposition_and_reveals() -> Result<(), String> {
        let (mut app, primary_window) = bound_window_app(
            BoundDisplay::Absent,
            BoundWindow::Primary,
            BoundConfigurationHistory::LoadedAtStartup,
            RecoveryPolicy::ReapplyOnReturn,
        )?;

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        assert!(entity_is_visible(&app, primary_window));
        assert_eq!(
            app.world().get::<WindowRevealDisposition>(primary_window),
            Some(&WindowRevealDisposition::FittedToFallbackDisplay)
        );
        Ok(())
    }

    #[test]
    fn unbound_saved_display_fits_to_fallback_and_reveals() -> Result<(), String> {
        let (mut app, primary_window) = unbound_saved_primary_window_app()?;
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create the primary role: {error}"))?;

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        let window = app
            .world()
            .get::<Window>(primary_window)
            .ok_or_else(|| String::from("the primary window disappeared"))?;
        assert!(window.visible);
        assert_eq!(window.resolution.physical_width(), SAVED_LOGICAL_WIDTH);
        assert_eq!(window.resolution.physical_height(), SAVED_LOGICAL_HEIGHT);
        assert_eq!(
            app.world().get::<WindowRevealDisposition>(primary_window),
            Some(&WindowRevealDisposition::FittedToFallbackDisplay)
        );
        assert_eq!(
            recovery_phase(&app),
            WindowFallbackRecoveryProgress::Recovering(
                WindowFallbackRecoveryPhase::MissingLiveMonitor
            )
        );
        assert!(
            app.world()
                .resource::<StrandedWindowMovementBaselines>()
                .is_tracked(&role)
        );
        Ok(())
    }

    #[test]
    fn absent_saved_display_without_current_monitor_abandons_placement_and_reveals()
    -> Result<(), String> {
        let (mut app, primary_window) = bound_window_app(
            BoundDisplay::Absent,
            BoundWindow::Primary,
            BoundConfigurationHistory::LoadedAtStartup,
            RecoveryPolicy::ReapplyOnReturn,
        )?;
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create the primary role: {error}"))?;
        app.world_mut()
            .entity_mut(primary_window)
            .remove::<CurrentMonitor>();

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        assert!(entity_is_visible(&app, primary_window));
        assert!(!has_reveal_wait(&app, primary_window));
        assert_eq!(
            app.world().get::<WindowRevealDisposition>(primary_window),
            Some(&WindowRevealDisposition::PlacementAbandoned)
        );
        assert_eq!(
            recovery_phase(&app),
            WindowFallbackRecoveryProgress::Recovering(
                WindowFallbackRecoveryPhase::MissingLiveMonitor
            )
        );
        assert!(
            app.world()
                .resource::<StrandedWindowMovementBaselines>()
                .is_tracked(&role)
        );
        Ok(())
    }

    #[test]
    fn fallback_placement_adopts_the_live_display_after_a_move() -> Result<(), String> {
        let (mut app, primary_window) = bound_window_app(
            BoundDisplay::Absent,
            BoundWindow::Primary,
            BoundConfigurationHistory::LoadedAtStartup,
            RecoveryPolicy::ReapplyOnReturn,
        )?;
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create the primary role: {error}"))?;
        let saved_placement = match app
            .world()
            .resource::<PersistedWindowPlacements>()
            .get(&role)
        {
            PersistedWindowPlacementLookup::Saved(placement) => placement,
            PersistedWindowPlacementLookup::NotSaved => {
                return Err(String::from("the saved primary record disappeared"));
            },
        };
        let saved_target = match &saved_placement.target {
            PersistedWindowTargetV5::Classified(device) => device.clone(),
            PersistedWindowTargetV5::AwaitingLegacyEvidence(_) => {
                return Err(String::from("the test target was not classified"));
            },
        };
        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        let window = app
            .world()
            .get::<Window>(primary_window)
            .ok_or_else(|| String::from("the primary window disappeared"))?;
        assert!(window.visible);
        assert_eq!(window.resolution.physical_width(), SAVED_LOGICAL_WIDTH);
        assert_eq!(window.resolution.physical_height(), SAVED_LOGICAL_HEIGHT);
        assert_eq!(window.position, WindowPosition::At(SAVED_WINDOW_OFFSET));
        assert_eq!(
            app.world()
                .resource::<Bindings>()
                .binding(&role)
                .map_err(|error| format!("the primary binding disappeared: {error}"))?
                .endpoint
                .device,
            saved_target
        );
        observe_fallback_baseline(&mut app, &role)?;
        let saved_placement = match app
            .world()
            .resource::<PersistedWindowPlacements>()
            .get(&role)
        {
            PersistedWindowPlacementLookup::Saved(placement) => placement,
            PersistedWindowPlacementLookup::NotSaved => {
                return Err(String::from("the saved primary record disappeared"));
            },
        };
        assert!(matches!(
            &saved_placement.target,
            PersistedWindowTargetV5::Classified(device) if device == &saved_target
        ));

        let (_, _, live_target) = test_display_devices()?;
        app.world_mut()
            .get_mut::<Window>(primary_window)
            .ok_or_else(|| String::from("the primary window disappeared"))?
            .position = WindowPosition::At(SAVED_WINDOW_OFFSET + IVec2::new(80, 60));
        app.world_mut()
            .run_system_once(managed::adopt_live_display_for_stranded_window)
            .map_err(|error| format!("failed to adopt the live display: {error}"))?;

        assert_eq!(
            app.world()
                .resource::<Bindings>()
                .binding(&role)
                .map_err(|error| format!("the adopted primary binding disappeared: {error}"))?
                .endpoint
                .device,
            live_target
        );
        Ok(())
    }

    #[test]
    fn active_only_projection_preserves_absent_display_recovery_from_startup_record()
    -> Result<(), String> {
        let (mut app, primary_window) = bound_window_app(
            BoundDisplay::Absent,
            BoundWindow::Primary,
            BoundConfigurationHistory::LoadedAtStartup,
            RecoveryPolicy::ReapplyOnReturn,
        )?;
        let _state_directory = install_active_only_persistence_writer(&mut app)?;
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create the primary role: {error}"))?;

        advance(
            &mut app,
            EXACT_DISPLAY_WAIT_TIMEOUT_SECS * PARTIAL_WAIT_FRACTION,
        );

        let persisted = app.world().resource::<PersistedWindowPlacements>();
        assert!(persisted.get(&role).is_saved());
        assert!(persisted.has_saved_configuration(&role));
        let binding = app
            .world()
            .resource::<Bindings>()
            .binding(&role)
            .map_err(|error| format!("primary binding disappeared: {error}"))?;
        assert!(matches!(
            &binding.last_known_good,
            LastKnownGoodConfiguration::NotEstablished
        ));
        assert!(!entity_is_visible(&app, primary_window));

        advance(
            &mut app,
            EXACT_DISPLAY_WAIT_TIMEOUT_SECS * PARTIAL_WAIT_FRACTION,
        );

        let window = app
            .world()
            .get::<Window>(primary_window)
            .ok_or_else(|| String::from("the primary window disappeared"))?;
        assert!(window.visible);
        assert_eq!(window.resolution.physical_width(), SAVED_LOGICAL_WIDTH);
        assert_eq!(window.resolution.physical_height(), SAVED_LOGICAL_HEIGHT);
        assert_eq!(
            recovery_phase(&app),
            WindowFallbackRecoveryProgress::Recovering(
                WindowFallbackRecoveryPhase::MissingLiveMonitor
            )
        );
        assert!(
            app.world()
                .resource::<StrandedWindowMovementBaselines>()
                .is_tracked(&role)
        );
        Ok(())
    }

    #[test]
    fn active_only_projection_preserves_absent_display_recovery_from_mid_session_save()
    -> Result<(), String> {
        let (mut app, primary_window) = bound_window_app(
            BoundDisplay::Live,
            BoundWindow::Primary,
            BoundConfigurationHistory::SavedDuringSession,
            RecoveryPolicy::ReapplyOnReturn,
        )?;
        let _state_directory = install_active_only_persistence_writer(&mut app)?;
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create the primary role: {error}"))?;

        let persisted = app.world().resource::<PersistedWindowPlacements>();
        assert!(persisted.get(&role).is_not_saved());
        assert!(!persisted.has_saved_configuration(&role));
        let binding = app
            .world()
            .resource::<Bindings>()
            .binding(&role)
            .map_err(|error| format!("primary binding disappeared: {error}"))?;
        let role_entity = app
            .world()
            .resource::<Bindings>()
            .role_entity(&role)
            .map_err(|error| format!("primary role entity disappeared: {error}"))?;
        assert!(matches!(
            app.world()
                .get::<RoleStatus>(role_entity)
                .map(RoleStatus::view),
            Some(RoleStatusView::Established { .. })
        ));
        assert!(binding.last_known_good().is_ok());

        app.update();

        let persisted = app.world().resource::<PersistedWindowPlacements>();
        assert!(persisted.get(&role).is_saved());
        assert!(persisted.has_saved_configuration(&role));

        report_bound_display(&mut app, BoundDisplay::Absent)?;
        app.world_mut()
            .run_system_once(persistence::write_established_window_configurations_for_test)
            .map_err(|error| format!("absent-display persistence write failed: {error}"))?;

        let persisted = app.world().resource::<PersistedWindowPlacements>();
        assert!(persisted.get(&role).is_saved());
        assert!(persisted.has_saved_configuration(&role));
        let binding = app
            .world()
            .resource::<Bindings>()
            .binding(&role)
            .map_err(|error| format!("primary binding disappeared: {error}"))?;
        let role_entity = app
            .world()
            .resource::<Bindings>()
            .role_entity(&role)
            .map_err(|error| format!("primary role entity disappeared: {error}"))?;
        assert!(matches!(
            app.world()
                .get::<RoleStatus>(role_entity)
                .map(RoleStatus::view),
            Some(RoleStatusView::Waiting(_))
        ));
        assert!(binding.last_known_good().is_ok());
        assert!(!entity_is_visible(&app, primary_window));

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(EXACT_DISPLAY_WAIT_TIMEOUT_SECS));
        app.world_mut()
            .run_system_once(abandon_placement_after_deadline)
            .map_err(|error| format!("placement deadline failed: {error}"))?;
        app.world_mut()
            .run_system_once(show_window_once_placement_settles)
            .map_err(|error| format!("window reveal failed: {error}"))?;

        let window = app
            .world()
            .get::<Window>(primary_window)
            .ok_or_else(|| String::from("the primary window disappeared"))?;
        assert!(window.visible);
        assert_eq!(window.resolution.physical_width(), SAVED_LOGICAL_WIDTH);
        assert_eq!(window.resolution.physical_height(), SAVED_LOGICAL_HEIGHT);
        assert_eq!(
            recovery_phase(&app),
            WindowFallbackRecoveryProgress::Recovering(
                WindowFallbackRecoveryPhase::MissingLiveMonitor
            )
        );
        assert!(
            app.world()
                .resource::<StrandedWindowMovementBaselines>()
                .is_tracked(&role)
        );
        Ok(())
    }

    #[test]
    fn reveals_a_hidden_managed_window_once_its_own_wait_expires() -> Result<(), String> {
        let mut app = reveal_app();
        let managed_window = app
            .world_mut()
            .spawn((
                Window {
                    visible: false,
                    ..default()
                },
                ManagedWindowName("inspector".into()),
                SavedDisplayRevealWait::default(),
            ))
            .id();
        let role = persistence::managed_window_role("inspector")
            .map_err(|error| format!("failed to create managed role: {error}"))?;

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        assert!(entity_is_visible(&app, managed_window));
        assert!(!has_reveal_wait(&app, managed_window));
        assert_eq!(
            recovery_phase_for_role(&app, &role),
            WindowFallbackRecoveryProgress::NotRecovering
        );
        Ok(())
    }

    #[test]
    fn managed_windows_wait_independently() {
        let mut app = reveal_app();
        let primary_window = app
            .world_mut()
            .spawn((
                Window {
                    visible: false,
                    ..default()
                },
                PrimaryWindow,
                SavedDisplayRevealWait::default(),
            ))
            .id();

        advance(
            &mut app,
            EXACT_DISPLAY_WAIT_TIMEOUT_SECS * PARTIAL_WAIT_FRACTION,
        );
        let managed_window = app
            .world_mut()
            .spawn((
                Window {
                    visible: false,
                    ..default()
                },
                ManagedWindowName("late-inspector".into()),
                SavedDisplayRevealWait::default(),
            ))
            .id();
        advance(
            &mut app,
            EXACT_DISPLAY_WAIT_TIMEOUT_SECS * PARTIAL_WAIT_FRACTION,
        );

        assert!(entity_is_visible(&app, primary_window));
        assert!(!entity_is_visible(&app, managed_window));
        assert!(has_reveal_wait(&app, managed_window));

        advance(
            &mut app,
            EXACT_DISPLAY_WAIT_TIMEOUT_SECS * PARTIAL_WAIT_FRACTION,
        );
        assert!(entity_is_visible(&app, managed_window));
    }

    #[test]
    fn a_respawned_window_receives_a_fresh_full_wait() {
        let mut app = reveal_app();
        app.add_observer(hide_window_on_creation);
        let original = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        app.world_mut().flush();

        advance(
            &mut app,
            EXACT_DISPLAY_WAIT_TIMEOUT_SECS * PARTIAL_WAIT_FRACTION,
        );
        assert!(app.world_mut().despawn(original));
        let respawned = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        app.world_mut().flush();
        assert!(!entity_is_visible(&app, respawned));
        assert!(has_reveal_wait(&app, respawned));

        advance(
            &mut app,
            EXACT_DISPLAY_WAIT_TIMEOUT_SECS * PARTIAL_WAIT_FRACTION,
        );
        assert!(!entity_is_visible(&app, respawned));

        advance(
            &mut app,
            EXACT_DISPLAY_WAIT_TIMEOUT_SECS * PARTIAL_WAIT_FRACTION,
        );
        assert!(entity_is_visible(&app, respawned));
    }

    #[test]
    fn authoritative_absence_reveals_without_attempt_finished_events() -> Result<(), String> {
        let mut app = bound_primary_window_app(BoundDisplay::Absent)?;
        for _ in 0..RECONSIDERATION_ROUNDS {
            advance(
                &mut app,
                EXACT_DISPLAY_WAIT_TIMEOUT_SECS * RECONSIDERATION_ROUND_FRACTION,
            );
        }

        assert!(window_is_visible(&mut app));
        assert_eq!(app.world().resource::<EndedAttemptEvents>().0, 0);
        Ok(())
    }

    #[test]
    fn records_no_recovery_phase_for_a_window_that_was_never_hidden() {
        let mut app = primary_window_app(true);

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        assert!(window_is_visible(&mut app));
        assert_eq!(
            recovery_phase(&app),
            WindowFallbackRecoveryProgress::NotRecovering
        );
        let primary_window = app
            .world_mut()
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .single(app.world())
            .expect("the application holds one primary window");
        assert!(!has_reveal_wait(&app, primary_window));
    }
}
