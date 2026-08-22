//! Window-only remainder of fallback-and-return recovery.

use std::collections::HashMap;

use bevy::prelude::On;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use hana_rigging::prelude::RetireRole;
use hana_rigging::prelude::RoleKey;

use crate::persistence::EstablishedWindowPlacement;

/// Window-specific progress that has no meaning for a generic device binding.
///
/// The kernel owns whether the role waits, applies, or retires. This state records only the
/// compositor-specific interval between losing the requested display and settling back onto it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowFallbackRecoveryPhase {
    /// The requested display is absent, so no exact live monitor target exists.
    MissingLiveMonitor,
    /// The compositor has placed the window on a temporary display.
    OnFallbackDisplay,
    /// The requested display returned and the compositor is settling the window there.
    SettlingOnRequestedDisplay,
}

/// Private per-role state for the window behavior layered on `ReapplyOnReturn`.
///
/// This resource deliberately contains no attempt identifier, device identity, captured
/// configuration, or binding state. Those facts stay in the rigging kernel so a window cannot
/// continue an operation the kernel has superseded.
#[derive(Default, Resource)]
pub(crate) struct WindowFallbackRecoveryState {
    phases: HashMap<RoleKey, WindowFallbackRecoveryPhase>,
}

impl WindowFallbackRecoveryState {
    pub(crate) fn mark_missing(&mut self, role: RoleKey) {
        self.phases
            .insert(role, WindowFallbackRecoveryPhase::MissingLiveMonitor);
    }

    pub(crate) fn mark_on_fallback(&mut self, role: RoleKey) {
        self.phases
            .insert(role, WindowFallbackRecoveryPhase::OnFallbackDisplay);
    }

    pub(crate) fn mark_settling(&mut self, role: RoleKey) {
        if let Some(phase) = self.phases.get_mut(&role) {
            *phase = WindowFallbackRecoveryPhase::SettlingOnRequestedDisplay;
        }
    }

    pub(crate) fn finish(&mut self, role: &RoleKey) { self.phases.remove(role); }

    pub(super) fn retire(&mut self, role: &RoleKey) { self.phases.remove(role); }

    #[cfg(test)]
    pub(crate) fn phase(&self, role: &RoleKey) -> Option<WindowFallbackRecoveryPhase> {
        self.phases.get(role).copied()
    }
}

/// Geometry a window came to rest at after the stranded-display fallback revealed it.
///
/// A role whose saved display is absent keeps its binding pointed at that display, so the window
/// still returns when the display does. The price is that the role never becomes ready, and
/// `write_established_window_configurations` writes nothing for a role that is not ready: while
/// the window is stranded, nothing the user does to it is saved. Moving the window is how the user
/// overrides that pending return, and this is what tells such a move apart from the fallback's own
/// placement.
///
/// The baseline is deliberately not the geometry the fallback requested. A compositor is free to
/// nudge a window it has just been handed, and reading that nudge as a move would throw away the
/// saved display the instant the window appeared. The baseline is the first geometry observed
/// twice in a row — the window at rest, wherever it actually landed.
#[derive(Default, Resource)]
pub(crate) struct StrandedWindowPlacements {
    entries: HashMap<RoleKey, StrandedWindowPlacement>,
}

/// Progress towards a baseline for one stranded window.
enum StrandedWindowPlacement {
    /// Revealed, but not yet seen at the same geometry on two consecutive observations.
    Settling(Option<EstablishedWindowPlacement>),
    /// Geometry the window settled at; a later difference is the user moving it.
    AtRest(EstablishedWindowPlacement),
}

/// What one readback of a stranded window says about who put it where it sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StrandedWindowObservation {
    /// The role is untracked, still settling, or still at rest where the fallback left it.
    Unmoved,
    /// The window left its resting geometry, so the user placed it where it now sits.
    Moved,
}

impl StrandedWindowPlacements {
    /// Start tracking a role whose window the fallback has just revealed.
    pub(crate) fn begin(&mut self, role: RoleKey) {
        self.entries
            .insert(role, StrandedWindowPlacement::Settling(None));
    }

    /// Whether no role is currently stranded, so per-window readback can be skipped entirely.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool { self.entries.is_empty() }

    /// Fold one live readback into a role's baseline and report what it means.
    pub(crate) fn observe(
        &mut self,
        role: &RoleKey,
        live: &EstablishedWindowPlacement,
    ) -> StrandedWindowObservation {
        let Some(entry) = self.entries.get_mut(role) else {
            return StrandedWindowObservation::Unmoved;
        };
        let settled = matches!(
            entry,
            StrandedWindowPlacement::Settling(Some(observed)) if observed == live
        );
        match entry {
            StrandedWindowPlacement::Settling(_) if settled => {
                *entry = StrandedWindowPlacement::AtRest(live.clone());
                StrandedWindowObservation::Unmoved
            },
            StrandedWindowPlacement::Settling(observed) => {
                *observed = Some(live.clone());
                StrandedWindowObservation::Unmoved
            },
            StrandedWindowPlacement::AtRest(at_rest) if at_rest == live => {
                StrandedWindowObservation::Unmoved
            },
            StrandedWindowPlacement::AtRest(_) => StrandedWindowObservation::Moved,
        }
    }

    /// Stop tracking a role, whether its window was adopted, retired, or carried home.
    pub(crate) fn forget(&mut self, role: &RoleKey) { self.entries.remove(role); }

    #[cfg(test)]
    pub(crate) fn is_tracked(&self, role: &RoleKey) -> bool { self.entries.contains_key(role) }
}

pub(super) fn on_retire_role(
    retire_role: On<RetireRole>,
    mut state: ResMut<WindowFallbackRecoveryState>,
    mut stranded: ResMut<StrandedWindowPlacements>,
) {
    state.retire(&retire_role.role);
    stranded.forget(&retire_role.role);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;

    use bevy::MinimalPlugins;
    use bevy::prelude::App;
    use bevy::prelude::Component;
    use bevy::prelude::Entity;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectComponent;
    use bevy::prelude::World;
    use hana_rigging::WaitingWork;
    use hana_rigging::prelude::ApplyDeadline;
    use hana_rigging::prelude::ApplyPermit;
    use hana_rigging::prelude::AttachmentPath;
    use hana_rigging::prelude::AttemptId;
    use hana_rigging::prelude::AttemptOutcome;
    use hana_rigging::prelude::AttemptProgress;
    use hana_rigging::prelude::AuthoritativeReporterCoverage;
    use hana_rigging::prelude::Binding;
    use hana_rigging::prelude::BindingEntities;
    use hana_rigging::prelude::BindingEntityLookup;
    use hana_rigging::prelude::Bindings;
    use hana_rigging::prelude::Capabilities;
    use hana_rigging::prelude::CaptureOutcome;
    use hana_rigging::prelude::Claim;
    use hana_rigging::prelude::CoveredDeviceIdentitySpace;
    use hana_rigging::prelude::DeviceDescriptor;
    use hana_rigging::prelude::DeviceEndpoint;
    use hana_rigging::prelude::DeviceIdSource;
    use hana_rigging::prelude::DeviceKey;
    use hana_rigging::prelude::DeviceKind;
    use hana_rigging::prelude::DeviceRecord;
    use hana_rigging::prelude::DeviceReporter;
    use hana_rigging::prelude::DeviceScan;
    use hana_rigging::prelude::DiscoveryCadence;
    use hana_rigging::prelude::DiscoveryWork;
    use hana_rigging::prelude::DriverId;
    use hana_rigging::prelude::EndpointDriver;
    use hana_rigging::prelude::EndpointId;
    use hana_rigging::prelude::LastKnownGoodConfiguration;
    use hana_rigging::prelude::MainThreadDiscoveryJob;
    use hana_rigging::prelude::OnAbort;
    use hana_rigging::prelude::OnSessionLoss;
    use hana_rigging::prelude::PlatformDeviceHandle;
    use hana_rigging::prelude::Presence;
    use hana_rigging::prelude::ReapplyConfiguration;
    use hana_rigging::prelude::RecoveryPolicy;
    use hana_rigging::prelude::ReportedAs;
    use hana_rigging::prelude::ReportedId;
    use hana_rigging::prelude::ReportedParent;
    use hana_rigging::prelude::ReportedSerial;
    use hana_rigging::prelude::ReporterCoverage;
    use hana_rigging::prelude::ReporterRegistration;
    use hana_rigging::prelude::RequestedConfiguration;
    use hana_rigging::prelude::RetryOn;
    use hana_rigging::prelude::RiggingAppExt;
    use hana_rigging::prelude::RiggingPlugin;
    use hana_rigging::prelude::RoleAvailable;
    use hana_rigging::prelude::RoleAwaiting;
    use hana_rigging::prelude::RoleState;
    use hana_rigging::prelude::SchemeName;

    use super::*;
    use crate::recovery::RecoveryPlugin;

    const FRAME_CEILING: usize = 32;

    #[derive(Clone, Component, Debug, PartialEq, Eq, Reflect)]
    #[reflect(Component, PartialEq)]
    struct TestWindowConfiguration(u8);

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct AppliedConfiguration {
        device: DeviceKey,
        value:  u8,
    }

    struct TestWindowDriver(Arc<Mutex<Vec<AppliedConfiguration>>>);

    impl EndpointDriver for TestWindowDriver {
        type Configuration = TestWindowConfiguration;

        fn capture(
            &mut self,
            _: &mut World,
            _: &DeviceEndpoint,
        ) -> CaptureOutcome<Self::Configuration> {
            CaptureOutcome::Read(TestWindowConfiguration(7))
        }

        fn start_apply(
            &mut self,
            _: &mut World,
            endpoint: &DeviceEndpoint,
            configuration: &Self::Configuration,
            _: AttemptId,
            _: ApplyPermit,
        ) {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(AppliedConfiguration {
                    device: endpoint.device.clone(),
                    value:  configuration.0,
                });
        }

        fn poll(&mut self, _: &mut World, _: AttemptId) -> AttemptProgress {
            AttemptProgress::Finished(AttemptOutcome::Succeeded)
        }
    }

    struct DisplayReporter(Arc<Mutex<Vec<DeviceKey>>>);

    impl DeviceReporter for DisplayReporter {
        fn discover(&mut self) -> DiscoveryWork {
            let reported = Arc::clone(&self.0);
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(move |_| {
                let records = reported
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .cloned()
                    .map(display_record)
                    .collect();
                DeviceScan::Complete(records)
            }))
        }
    }

    #[derive(Default, Resource)]
    struct RecoveryFacts {
        awaiting:  Vec<RoleKey>,
        available: Vec<RoleKey>,
    }

    fn record_awaiting(role_awaiting: On<RoleAwaiting>, mut facts: ResMut<RecoveryFacts>) {
        facts.awaiting.push(role_awaiting.role.clone());
    }

    fn record_available(role_available: On<RoleAvailable>, mut facts: ResMut<RecoveryFacts>) {
        facts.available.push(role_available.role.clone());
    }

    struct RecoveryHarness {
        app:      App,
        reported: Arc<Mutex<Vec<DeviceKey>>>,
        applied:  Arc<Mutex<Vec<AppliedConfiguration>>>,
        driver:   DriverId,
    }

    impl RecoveryHarness {
        fn new() -> Result<Self, String> {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .add_plugins((RiggingPlugin, RecoveryPlugin))
                .register_device_scheme(display_scheme()?)
                .init_resource::<RecoveryFacts>()
                .add_observer(record_awaiting)
                .add_observer(record_available);
            let reported = Arc::new(Mutex::new(Vec::new()));
            app.add_device_reporter(
                DisplayReporter(Arc::clone(&reported)),
                ReporterRegistration::required(
                    DiscoveryCadence::Periodic {
                        interval: Duration::ZERO,
                    },
                    ReporterCoverage::EstablishesAbsence(AuthoritativeReporterCoverage::one(
                        CoveredDeviceIdentitySpace::AllKeysOfKind {
                            kind: DeviceKind::Display,
                        },
                    )),
                ),
            );
            let applied = Arc::new(Mutex::new(Vec::new()));
            let driver = app.add_endpoint_driver(TestWindowDriver(Arc::clone(&applied)));

            Ok(Self {
                app,
                reported,
                applied,
                driver,
            })
        }

        fn bind(
            &mut self,
            role_name: &str,
            device_name: &str,
            recovery_policy: RecoveryPolicy,
        ) -> Result<(RoleKey, DeviceKey), String> {
            let role = RoleKey::new(role_name)
                .map_err(|error| format!("failed to create recovery role: {error}"))?;
            let device_key = display_key(device_name)?;
            self.app
                .world_mut()
                .resource_mut::<Bindings>()
                .register(test_binding(
                    role.clone(),
                    device_key.clone(),
                    self.driver,
                    recovery_policy,
                ))
                .map_err(|error| format!("failed to register recovery binding: {error}"))?;
            Ok((role, device_key))
        }

        fn report(&self, device_keys: &[DeviceKey]) {
            *self
                .reported
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = device_keys.to_vec();
        }

        fn advance(&mut self, frames: usize) {
            for _ in 0..frames {
                self.app.update();
            }
        }

        fn reach_role_state(
            &mut self,
            role: &RoleKey,
            role_state: RoleState,
        ) -> Result<(), String> {
            for _ in 0..FRAME_CEILING {
                if self
                    .app
                    .world()
                    .resource::<Bindings>()
                    .binding(role)
                    .is_ok_and(|binding| binding.state == role_state)
                {
                    return Ok(());
                }
                self.app.update();
            }
            Err(format!(
                "role {role:?} did not reach {role_state:?} within {FRAME_CEILING} frames"
            ))
        }

        fn reach_waiting_work(
            &mut self,
            role: &RoleKey,
            waiting_work: WaitingWork,
        ) -> Result<(), String> {
            for _ in 0..FRAME_CEILING {
                if self.app.world().resource::<Bindings>().waiting_work(role) == waiting_work {
                    return Ok(());
                }
                self.app.update();
            }
            Err(format!(
                "role {role:?} did not reach {waiting_work:?} within {FRAME_CEILING} frames"
            ))
        }

        fn reach_apply(&mut self, device_key: &DeviceKey) -> Result<(), String> {
            for _ in 0..FRAME_CEILING {
                if !self.applied_values(device_key).is_empty() {
                    return Ok(());
                }
                self.app.update();
            }
            Err(format!(
                "device {device_key:?} was not applied within {FRAME_CEILING} frames"
            ))
        }

        fn binding_entity(&self, role: &RoleKey) -> Result<Entity, String> {
            match self.app.world().resource::<BindingEntities>().entity(role) {
                BindingEntityLookup::Registered(entity) => Ok(entity),
                BindingEntityLookup::Unregistered => {
                    Err(format!("role {role:?} has no registered binding entity"))
                },
            }
        }

        fn clear_applied(&self) {
            self.applied
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();
        }

        fn applied_values(&self, device_key: &DeviceKey) -> Vec<u8> {
            self.applied
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .filter(|applied| &applied.device == device_key)
                .map(|applied| applied.value)
                .collect()
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum LastKnownGoodStatus {
        Known,
        NotEstablished,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct RecoveryPolicyObservation {
        waiting_work:    WaitingWork,
        last_known_good: LastKnownGoodStatus,
        return_apply:    Vec<u8>,
    }

    fn display_scheme() -> Result<SchemeName, String> {
        SchemeName::new("clerestory-recovery-test")
            .map_err(|error| format!("failed to create recovery display scheme: {error}"))
    }

    fn display_key(value: &str) -> Result<DeviceKey, String> {
        Ok(DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Reported {
                scheme: display_scheme()?,
                value:  ReportedId::new(value)
                    .map_err(|error| format!("failed to create recovery display ID: {error}"))?,
            },
        })
    }

    fn display_record(device_key: DeviceKey) -> DeviceRecord {
        DeviceRecord {
            reported_as:            ReportedAs::Keyed(device_key),
            parent:                 ReportedParent::Root,
            presence:               Presence::Present,
            claim:                  Claim::NotApplicable,
            capabilities:           Capabilities::new(),
            serial:                 ReportedSerial::NotExposedByUnit,
            platform_device_handle: PlatformDeviceHandle::PlatformReportedNothing,
            attachment:             AttachmentPath::PlatformHasNoConcept,
            descriptor:             DeviceDescriptor::PlatformReportedNothing,
        }
    }

    fn test_binding(
        role: RoleKey,
        device_key: DeviceKey,
        driver: DriverId,
        recovery_policy: RecoveryPolicy,
    ) -> Binding {
        Binding {
            role,
            endpoint: DeviceEndpoint {
                device: device_key,
                id:     EndpointId::Whole,
            },
            driver,
            recovery: recovery_policy,
            retry: RetryOn::NewRevision,
            on_abort: OnAbort::default(),
            on_loss: OnSessionLoss::default(),
            state: RoleState::Waiting,
            requested: RequestedConfiguration::new(TestWindowConfiguration(3)),
            last_known_good: LastKnownGoodConfiguration::known(TestWindowConfiguration(7)),
            apply_deadline: ApplyDeadline::ProcessDefault,
        }
    }

    fn observe_policy(
        recovery_policy: RecoveryPolicy,
    ) -> Result<RecoveryPolicyObservation, String> {
        let mut harness = RecoveryHarness::new()?;
        let (role, device_key) =
            harness.bind("window:managed:policy", "policy-display", recovery_policy)?;
        harness.report(std::slice::from_ref(&device_key));
        harness.reach_role_state(&role, RoleState::Ready)?;
        harness.clear_applied();

        harness.report(&[]);
        let expected_work = match recovery_policy {
            RecoveryPolicy::ReapplyOnReturn => WaitingWork::RestorationOwed,
            RecoveryPolicy::Forget | RecoveryPolicy::Retain | RecoveryPolicy::ReapplyOnRequest => {
                WaitingWork::ApplicationRequestOwed
            },
        };
        harness.reach_waiting_work(&role, expected_work)?;
        let (waiting_work, last_known_good) = {
            let bindings = harness.app.world().resource::<Bindings>();
            let last_known_good = if matches!(
                &bindings
                    .binding(&role)
                    .map_err(|error| format!("recovery binding disappeared: {error}"))?
                    .last_known_good,
                LastKnownGoodConfiguration::Known(_)
            ) {
                LastKnownGoodStatus::Known
            } else {
                LastKnownGoodStatus::NotEstablished
            };
            (bindings.waiting_work(&role), last_known_good)
        };

        harness.report(std::slice::from_ref(&device_key));
        match recovery_policy {
            RecoveryPolicy::ReapplyOnReturn => {
                harness.reach_apply(&device_key)?;
            },
            RecoveryPolicy::Forget | RecoveryPolicy::Retain | RecoveryPolicy::ReapplyOnRequest => {
                harness.advance(FRAME_CEILING);
            },
        }
        Ok(RecoveryPolicyObservation {
            waiting_work,
            last_known_good,
            return_apply: harness.applied_values(&device_key),
        })
    }

    #[test]
    fn recovery_policies_keep_distinct_departure_and_return_behavior() -> Result<(), String> {
        let forget = observe_policy(RecoveryPolicy::Forget)?;
        let retain = observe_policy(RecoveryPolicy::Retain)?;
        let reapply_on_request = observe_policy(RecoveryPolicy::ReapplyOnRequest)?;
        let reapply_on_return = observe_policy(RecoveryPolicy::ReapplyOnReturn)?;

        assert_eq!(
            forget,
            RecoveryPolicyObservation {
                waiting_work:    WaitingWork::ApplicationRequestOwed,
                last_known_good: LastKnownGoodStatus::NotEstablished,
                return_apply:    Vec::new(),
            }
        );
        assert_eq!(
            retain,
            RecoveryPolicyObservation {
                waiting_work:    WaitingWork::ApplicationRequestOwed,
                last_known_good: LastKnownGoodStatus::Known,
                return_apply:    Vec::new(),
            }
        );
        assert_eq!(
            reapply_on_request,
            RecoveryPolicyObservation {
                waiting_work:    WaitingWork::ApplicationRequestOwed,
                last_known_good: LastKnownGoodStatus::Known,
                return_apply:    Vec::new(),
            }
        );
        assert_eq!(
            reapply_on_return,
            RecoveryPolicyObservation {
                waiting_work:    WaitingWork::RestorationOwed,
                last_known_good: LastKnownGoodStatus::Known,
                return_apply:    vec![7],
            }
        );
        Ok(())
    }

    #[test]
    fn reapply_request_requires_departure_debt_on_its_binding_entity() -> Result<(), String> {
        let mut harness = RecoveryHarness::new()?;
        let (first_role, first_device) = harness.bind(
            "window:managed:first",
            "first-display",
            RecoveryPolicy::ReapplyOnRequest,
        )?;
        let (second_role, second_device) = harness.bind(
            "window:managed:second",
            "second-display",
            RecoveryPolicy::ReapplyOnRequest,
        )?;
        harness.report(&[first_device.clone(), second_device.clone()]);
        harness.reach_role_state(&first_role, RoleState::Ready)?;
        harness.reach_role_state(&second_role, RoleState::Ready)?;
        let first_entity = harness.binding_entity(&first_role)?;
        let second_entity = harness.binding_entity(&second_role)?;
        harness.clear_applied();

        harness.app.world_mut().trigger(ReapplyConfiguration {
            binding: first_entity,
        });
        harness.advance(2);
        assert!(harness.applied_values(&first_device).is_empty());

        harness.report(std::slice::from_ref(&second_device));
        harness.reach_waiting_work(&first_role, WaitingWork::ApplicationRequestOwed)?;
        assert_eq!(
            harness
                .app
                .world()
                .resource::<Bindings>()
                .waiting_work(&first_role),
            WaitingWork::ApplicationRequestOwed
        );
        harness.report(&[first_device.clone(), second_device.clone()]);
        harness.advance(4);
        assert!(harness.applied_values(&first_device).is_empty());

        harness.app.world_mut().trigger(ReapplyConfiguration {
            binding: second_entity,
        });
        harness.advance(2);
        assert!(harness.applied_values(&first_device).is_empty());
        assert!(harness.applied_values(&second_device).is_empty());

        harness.app.world_mut().trigger(ReapplyConfiguration {
            binding: first_entity,
        });
        harness.reach_apply(&first_device)?;
        assert_eq!(harness.applied_values(&first_device), vec![7]);
        assert!(harness.applied_values(&second_device).is_empty());
        Ok(())
    }

    fn assert_observed_once(observed: &[RoleKey], role: &RoleKey) {
        assert_eq!(
            observed.iter().filter(|observed| *observed == role).count(),
            1
        );
    }

    #[test]
    fn role_events_and_retirement_progress_two_roles_independently() -> Result<(), String> {
        let mut harness = RecoveryHarness::new()?;
        let (first_role, first_device) = harness.bind(
            "window:managed:first",
            "first-display",
            RecoveryPolicy::ReapplyOnReturn,
        )?;
        let (second_role, second_device) = harness.bind(
            "window:managed:second",
            "second-display",
            RecoveryPolicy::ReapplyOnReturn,
        )?;

        harness.advance(4);
        {
            let facts = harness.app.world().resource::<RecoveryFacts>();
            assert_observed_once(&facts.awaiting, &first_role);
            assert_observed_once(&facts.awaiting, &second_role);
        }
        {
            let mut state = harness
                .app
                .world_mut()
                .resource_mut::<WindowFallbackRecoveryState>();
            state.mark_missing(first_role.clone());
            state.mark_missing(second_role.clone());
        }
        let state = harness
            .app
            .world()
            .resource::<WindowFallbackRecoveryState>();
        assert_eq!(
            state.phase(&first_role),
            Some(WindowFallbackRecoveryPhase::MissingLiveMonitor)
        );
        assert_eq!(
            state.phase(&second_role),
            Some(WindowFallbackRecoveryPhase::MissingLiveMonitor)
        );

        harness.report(std::slice::from_ref(&first_device));
        harness.advance(6);
        {
            let facts = harness.app.world().resource::<RecoveryFacts>();
            assert_observed_once(&facts.available, &first_role);
            assert!(!facts.available.contains(&second_role));
        }

        harness.app.world_mut().trigger(RetireRole {
            role: first_role.clone(),
        });
        let state = harness
            .app
            .world()
            .resource::<WindowFallbackRecoveryState>();
        assert_eq!(state.phase(&first_role), None);
        assert_eq!(
            state.phase(&second_role),
            Some(WindowFallbackRecoveryPhase::MissingLiveMonitor)
        );
        {
            let bindings = harness.app.world().resource::<Bindings>();
            assert!(bindings.binding(&first_role).is_err());
            assert!(bindings.binding(&second_role).is_ok());
        }

        harness.advance(2);
        let binding_entities = harness.app.world().resource::<BindingEntities>();
        assert_eq!(
            binding_entities.entity(&first_role),
            BindingEntityLookup::Unregistered
        );
        assert!(matches!(
            binding_entities.entity(&second_role),
            BindingEntityLookup::Registered(_)
        ));

        harness.report(std::slice::from_ref(&second_device));
        harness.advance(6);
        let facts = harness.app.world().resource::<RecoveryFacts>();
        assert_observed_once(&facts.available, &second_role);
        assert!(
            harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&second_role)
                .is_ok()
        );
        Ok(())
    }
}
