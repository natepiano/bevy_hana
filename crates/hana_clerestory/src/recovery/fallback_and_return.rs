//! Window-only remainder of fallback-and-return recovery.

use std::collections::HashMap;

use bevy::prelude::On;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use hana_rigging::prelude::DeviceKey;
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

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowFallbackRecoveryProgress {
    NotRecovering,
    Recovering(WindowFallbackRecoveryPhase),
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
    pub(crate) fn phase(&self, role: &RoleKey) -> WindowFallbackRecoveryProgress {
        self.phases.get(role).copied().map_or(
            WindowFallbackRecoveryProgress::NotRecovering,
            WindowFallbackRecoveryProgress::Recovering,
        )
    }
}

/// Display and placement baselines that distinguish fallback settling from a user move.
///
/// Placement readback is display-relative: neither its device nor its geometry can answer the
/// movement question alone. One role therefore owns one entry containing both observations.
#[derive(Default, Resource)]
pub(crate) struct StrandedWindowMovementBaselines {
    entries: HashMap<RoleKey, StrandedWindowMovementBaseline>,
}

enum StrandedWindowMovementBaseline {
    /// Neither a display nor a placement observation has arrived yet.
    AwaitingDisplayAndPlacement,
    /// A display observation arrived before any placement observation.
    AwaitingPlacementObservation { display: DeviceKey },
    /// A placement observation arrived before any display observation.
    AwaitingDisplay {
        placement: EstablishedWindowPlacement,
    },
    /// One placement observation was recorded; a matching next one confirms it.
    AwaitingConfirmation {
        display:   DeviceKey,
        placement: EstablishedWindowPlacement,
    },
    /// Where the window settled; a later difference is the user moving it.
    Confirmed {
        display:   DeviceKey,
        placement: EstablishedWindowPlacement,
    },
}

/// Relationship between a stranded window's live display and its recorded baseline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StrandedWindowDisplayObservation {
    /// The role has no stranded-window movement baseline.
    NotTracked,
    /// This is the first exact live display observed for the stranded window.
    BaselineRecorded,
    /// The window remains on the exact live display previously observed under it.
    MatchesBaseline,
    /// The window now occupies a different exact live display.
    DiffersFromBaseline,
}

/// What one readback of a stranded window says about who put it where it sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StrandedWindowObservation {
    /// The role is untracked, still settling, or still at rest where the fallback left it.
    Unmoved,
    /// The window left its resting geometry, so the user placed it where it now sits.
    Moved,
}

impl StrandedWindowMovementBaselines {
    /// Start tracking a role whose window the fallback has just revealed.
    pub(crate) fn begin(&mut self, role: RoleKey) {
        self.entries.insert(
            role,
            StrandedWindowMovementBaseline::AwaitingDisplayAndPlacement,
        );
    }

    /// Restart placement tracking while retaining any exact live-display baseline.
    pub(crate) fn restart_placement(&mut self, role: RoleKey) {
        let baseline = self
            .entries
            .entry(role)
            .or_insert(StrandedWindowMovementBaseline::AwaitingDisplayAndPlacement);
        match baseline {
            StrandedWindowMovementBaseline::AwaitingDisplayAndPlacement
            | StrandedWindowMovementBaseline::AwaitingPlacementObservation { .. } => {},
            StrandedWindowMovementBaseline::AwaitingDisplay { .. } => {
                *baseline = StrandedWindowMovementBaseline::AwaitingDisplayAndPlacement;
            },
            StrandedWindowMovementBaseline::AwaitingConfirmation { display, .. }
            | StrandedWindowMovementBaseline::Confirmed { display, .. } => {
                *baseline = StrandedWindowMovementBaseline::AwaitingPlacementObservation {
                    display: display.clone(),
                };
            },
        }
    }

    /// Whether no role is currently stranded, so per-window readback can be skipped entirely.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool { self.entries.is_empty() }

    /// Record or compare the exact live display currently under a stranded window.
    pub(crate) fn observe_display(
        &mut self,
        role: &RoleKey,
        device: &DeviceKey,
    ) -> StrandedWindowDisplayObservation {
        let Some(entry) = self.entries.get_mut(role) else {
            return StrandedWindowDisplayObservation::NotTracked;
        };
        match entry {
            StrandedWindowMovementBaseline::AwaitingDisplayAndPlacement => {
                *entry = StrandedWindowMovementBaseline::AwaitingPlacementObservation {
                    display: device.clone(),
                };
                StrandedWindowDisplayObservation::BaselineRecorded
            },
            StrandedWindowMovementBaseline::AwaitingDisplay { placement } => {
                *entry = StrandedWindowMovementBaseline::AwaitingConfirmation {
                    display:   device.clone(),
                    placement: placement.clone(),
                };
                StrandedWindowDisplayObservation::BaselineRecorded
            },
            StrandedWindowMovementBaseline::AwaitingPlacementObservation { display }
            | StrandedWindowMovementBaseline::AwaitingConfirmation { display, .. }
            | StrandedWindowMovementBaseline::Confirmed { display, .. }
                if display == device =>
            {
                StrandedWindowDisplayObservation::MatchesBaseline
            },
            StrandedWindowMovementBaseline::AwaitingPlacementObservation { .. }
            | StrandedWindowMovementBaseline::AwaitingConfirmation { .. }
            | StrandedWindowMovementBaseline::Confirmed { .. } => {
                StrandedWindowDisplayObservation::DiffersFromBaseline
            },
        }
    }

    /// Fold one live readback into a role's baseline and report what it means.
    pub(crate) fn observe_placement(
        &mut self,
        role: &RoleKey,
        live: &EstablishedWindowPlacement,
    ) -> StrandedWindowObservation {
        let Some(entry) = self.entries.get_mut(role) else {
            return StrandedWindowObservation::Unmoved;
        };
        match entry {
            StrandedWindowMovementBaseline::AwaitingDisplayAndPlacement => {
                *entry = StrandedWindowMovementBaseline::AwaitingDisplay {
                    placement: live.clone(),
                };
                StrandedWindowObservation::Unmoved
            },
            StrandedWindowMovementBaseline::AwaitingPlacementObservation { display } => {
                *entry = StrandedWindowMovementBaseline::AwaitingConfirmation {
                    display:   display.clone(),
                    placement: live.clone(),
                };
                StrandedWindowObservation::Unmoved
            },
            StrandedWindowMovementBaseline::AwaitingConfirmation { display, placement }
                if placement == live =>
            {
                *entry = StrandedWindowMovementBaseline::Confirmed {
                    display:   display.clone(),
                    placement: live.clone(),
                };
                StrandedWindowObservation::Unmoved
            },
            StrandedWindowMovementBaseline::AwaitingDisplay { placement }
            | StrandedWindowMovementBaseline::AwaitingConfirmation { placement, .. } => {
                placement.clone_from(live);
                StrandedWindowObservation::Unmoved
            },
            StrandedWindowMovementBaseline::Confirmed { placement, .. } if placement == live => {
                StrandedWindowObservation::Unmoved
            },
            StrandedWindowMovementBaseline::Confirmed { .. } => StrandedWindowObservation::Moved,
        }
    }

    /// Stop tracking a role, whether its window was adopted, retired, or moved back to its saved
    /// display.
    pub(crate) fn forget(&mut self, role: &RoleKey) { self.entries.remove(role); }

    /// Whether the role still awaits either a settled fallback placement or a user move.
    #[must_use]
    pub(crate) fn is_tracked(&self, role: &RoleKey) -> bool { self.entries.contains_key(role) }
}

pub(super) fn on_retire_role(
    retire_role: On<RetireRole>,
    mut state: ResMut<WindowFallbackRecoveryState>,
    mut baselines: ResMut<StrandedWindowMovementBaselines>,
) {
    state.retire(&retire_role.role);
    baselines.forget(&retire_role.role);
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
    use bevy::prelude::IVec2;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectComponent;
    use bevy::prelude::UVec2;
    use bevy::prelude::World;
    use hana_rigging::WaitingWork;
    use hana_rigging::prelude::Applied;
    use hana_rigging::prelude::ApplyContext;
    use hana_rigging::prelude::ApplyDeadline;
    use hana_rigging::prelude::AttachmentPath;
    use hana_rigging::prelude::AttemptInvalidation;
    use hana_rigging::prelude::AttemptRef;
    use hana_rigging::prelude::AuthoritativeReporterCoverage;
    use hana_rigging::prelude::BindingAuthoring;
    use hana_rigging::prelude::BindingPolicy;
    use hana_rigging::prelude::Bindings;
    use hana_rigging::prelude::Capabilities;
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
    use hana_rigging::prelude::DriverCleanupRoleEntity;
    use hana_rigging::prelude::DriverCompletion;
    use hana_rigging::prelude::EndpointDriver;
    use hana_rigging::prelude::EndpointDriverRegistration;
    use hana_rigging::prelude::EndpointId;
    use hana_rigging::prelude::EstablishedContext;
    use hana_rigging::prelude::LiveRoleChange;
    use hana_rigging::prelude::LiveRoleChanged;
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
    use hana_rigging::prelude::RetryOn;
    use hana_rigging::prelude::RiggingAppExt;
    use hana_rigging::prelude::RiggingPlugin;
    use hana_rigging::prelude::RoleKey;
    use hana_rigging::prelude::RoleStatus;
    use hana_rigging::prelude::RoleStatusView;
    use hana_rigging::prelude::SchemeName;
    use hana_rigging::prelude::SessionRef;
    use hana_rigging::prelude::SessionReleaseCause;
    use hana_rigging::prelude::TargetResolution;
    use hana_rigging::prelude::TargetResolutionContext;
    use hana_rigging::prelude::WaitingStatusView;
    use hana_rigging::prelude::register_binding;

    use super::*;
    use crate::persistence::EstablishedWindowPosition;
    use crate::persistence::SavedWindowMode;
    use crate::recovery::RecoveryPlugin;

    const FRAME_CEILING: usize = 32;

    fn placement(logical_x: i32) -> EstablishedWindowPlacement {
        EstablishedWindowPlacement {
            position:          EstablishedWindowPosition::Restorable {
                logical_offset: IVec2::new(logical_x, 20),
            },
            logical_size:      UVec2::new(800, 600),
            saved_window_mode: SavedWindowMode::Windowed,
        }
    }

    #[test]
    fn display_first_arrival_builds_one_confirmed_movement_baseline() -> Result<(), String> {
        let role = RoleKey::new("window:managed:display-first")
            .map_err(|error| format!("failed to create role: {error}"))?;
        let display = display_key("display-first")?;
        let baseline = placement(10);
        let moved = placement(30);
        let mut baselines = StrandedWindowMovementBaselines::default();

        baselines.begin(role.clone());
        assert_eq!(
            baselines.observe_display(&role, &display),
            StrandedWindowDisplayObservation::BaselineRecorded
        );
        assert_eq!(
            baselines.observe_placement(&role, &baseline),
            StrandedWindowObservation::Unmoved
        );
        assert_eq!(
            baselines.observe_placement(&role, &baseline),
            StrandedWindowObservation::Unmoved
        );
        assert_eq!(
            baselines.observe_placement(&role, &moved),
            StrandedWindowObservation::Moved
        );
        Ok(())
    }

    #[test]
    fn placement_first_arrival_builds_one_confirmed_movement_baseline() -> Result<(), String> {
        let role = RoleKey::new("window:managed:placement-first")
            .map_err(|error| format!("failed to create role: {error}"))?;
        let display = display_key("placement-first")?;
        let other_display = display_key("placement-first-other")?;
        let baseline = placement(10);
        let mut baselines = StrandedWindowMovementBaselines::default();

        baselines.begin(role.clone());
        assert_eq!(
            baselines.observe_placement(&role, &baseline),
            StrandedWindowObservation::Unmoved
        );
        assert_eq!(
            baselines.observe_display(&role, &display),
            StrandedWindowDisplayObservation::BaselineRecorded
        );
        assert_eq!(
            baselines.observe_placement(&role, &baseline),
            StrandedWindowObservation::Unmoved
        );
        assert_eq!(
            baselines.observe_display(&role, &display),
            StrandedWindowDisplayObservation::MatchesBaseline
        );
        assert_eq!(
            baselines.observe_display(&role, &other_display),
            StrandedWindowDisplayObservation::DiffersFromBaseline
        );
        Ok(())
    }

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
            configuration: &Self::Configuration,
            (): Self::Target,
        ) {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(AppliedConfiguration {
                    device: context.target().endpoint().device.clone(),
                    value:  configuration.0,
                });
            context
                .into_completion()
                .finish(DriverCompletion::Succeeded(Applied::DiffersFromDispatched(
                    TestWindowConfiguration(7),
                )));
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

    fn record_role_availability(
        live_role_changed: On<LiveRoleChanged>,
        mut facts: ResMut<RecoveryFacts>,
    ) {
        let LiveRoleChange::Status { from, to } = &live_role_changed.change else {
            return;
        };
        let was_waiting = matches!(
            from.view(),
            RoleStatusView::Waiting(WaitingStatusView::Reporter(_))
        );
        let is_waiting = matches!(
            to.view(),
            RoleStatusView::Waiting(WaitingStatusView::Reporter(_))
        );
        match (was_waiting, is_waiting) {
            (false, true) => facts.awaiting.push(live_role_changed.role.clone()),
            (true, false) => facts.available.push(live_role_changed.role.clone()),
            (false, false) | (true, true) => {},
        }
    }

    struct RecoveryHarness {
        app:      App,
        reported: Arc<Mutex<Vec<DeviceKey>>>,
        applied:  Arc<Mutex<Vec<AppliedConfiguration>>>,
        driver:   EndpointDriverRegistration<TestWindowConfiguration>,
    }

    impl RecoveryHarness {
        fn new() -> Result<Self, String> {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .add_plugins((RiggingPlugin, RecoveryPlugin))
                .register_device_scheme(display_scheme()?)
                .init_resource::<RecoveryFacts>()
                .add_observer(record_role_availability);
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
                    std::time::Duration::from_secs(10),
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
            register_binding(
                self.app.world_mut(),
                test_binding(
                    role.clone(),
                    device_key.clone(),
                    self.driver,
                    recovery_policy,
                ),
            )
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

        fn reach_established(&mut self, role: &RoleKey) -> Result<(), String> {
            for _ in 0..FRAME_CEILING {
                let role_entity = self
                    .app
                    .world()
                    .resource::<Bindings>()
                    .role_entity(role)
                    .ok();
                if role_entity.is_some_and(|entity| {
                    self.app
                        .world()
                        .get::<RoleStatus>(entity)
                        .is_some_and(|status| {
                            matches!(status.view(), RoleStatusView::Established { .. })
                        })
                }) {
                    return Ok(());
                }
                self.app.update();
            }
            Err(format!(
                "role {role:?} did not become established within {FRAME_CEILING} frames"
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
            self.app
                .world()
                .resource::<Bindings>()
                .role_entity(role)
                .map_err(|_| format!("role {role:?} has no registered binding entity"))
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
            platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
            attachment:             AttachmentPath::PlatformHasNoConcept,
            descriptor:             DeviceDescriptor::PlatformHasNoConcept,
        }
    }

    fn test_binding(
        role: RoleKey,
        device_key: DeviceKey,
        driver: EndpointDriverRegistration<TestWindowConfiguration>,
        recovery_policy: RecoveryPolicy,
    ) -> BindingAuthoring<TestWindowConfiguration> {
        BindingAuthoring::new(
            role,
            DeviceEndpoint {
                device: device_key,
                id:     EndpointId::Whole,
            },
            driver,
            TestWindowConfiguration(3),
            BindingPolicy::new(
                recovery_policy,
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        )
    }

    fn observe_policy(
        recovery_policy: RecoveryPolicy,
    ) -> Result<RecoveryPolicyObservation, String> {
        let mut harness = RecoveryHarness::new()?;
        let (role, device_key) =
            harness.bind("window:managed:policy", "policy-display", recovery_policy)?;
        harness.report(std::slice::from_ref(&device_key));
        harness.reach_established(&role)?;
        harness.clear_applied();

        harness.report(&[]);
        let expected_work = match recovery_policy {
            RecoveryPolicy::ReapplyOnReturn => WaitingWork::RestorationOwed,
            RecoveryPolicy::ReapplyOnRequest => WaitingWork::ReapplyRequestOwed,
            RecoveryPolicy::Forget => WaitingWork::RegistrationOwed,
        };
        harness.reach_waiting_work(&role, expected_work)?;
        let (waiting_work, last_known_good) = {
            let bindings = harness.app.world().resource::<Bindings>();
            let last_known_good = if bindings
                .binding(&role)
                .map_err(|error| format!("recovery binding disappeared: {error}"))?
                .last_known_good()
                .is_ok()
            {
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
            RecoveryPolicy::Forget | RecoveryPolicy::ReapplyOnRequest => {
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
        let reapply_on_request = observe_policy(RecoveryPolicy::ReapplyOnRequest)?;
        let reapply_on_return = observe_policy(RecoveryPolicy::ReapplyOnReturn)?;

        // Every row differs from every other row in at least one column, which is what the name of
        // this test claims and what an enum of policies is for. A variant whose row duplicated
        // another's would produce the same behavior as that other variant, leaving the difference
        // between the two only in the documentation.
        assert_eq!(
            forget,
            RecoveryPolicyObservation {
                waiting_work:    WaitingWork::RegistrationOwed,
                last_known_good: LastKnownGoodStatus::NotEstablished,
                return_apply:    Vec::new(),
            }
        );
        assert_eq!(
            reapply_on_request,
            RecoveryPolicyObservation {
                waiting_work:    WaitingWork::ReapplyRequestOwed,
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
        harness.reach_established(&first_role)?;
        harness.reach_established(&second_role)?;
        let first_entity = harness.binding_entity(&first_role)?;
        let second_entity = harness.binding_entity(&second_role)?;
        harness.clear_applied();

        harness.app.world_mut().trigger(ReapplyConfiguration {
            binding: first_entity,
        });
        harness.advance(2);
        assert!(harness.applied_values(&first_device).is_empty());

        harness.report(std::slice::from_ref(&second_device));
        harness.reach_waiting_work(&first_role, WaitingWork::ReapplyRequestOwed)?;
        assert_eq!(
            harness
                .app
                .world()
                .resource::<Bindings>()
                .waiting_work(&first_role),
            WaitingWork::ReapplyRequestOwed
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
            WindowFallbackRecoveryProgress::Recovering(
                WindowFallbackRecoveryPhase::MissingLiveMonitor
            )
        );
        assert_eq!(
            state.phase(&second_role),
            WindowFallbackRecoveryProgress::Recovering(
                WindowFallbackRecoveryPhase::MissingLiveMonitor
            )
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
        assert_eq!(
            state.phase(&first_role),
            WindowFallbackRecoveryProgress::NotRecovering
        );
        assert_eq!(
            state.phase(&second_role),
            WindowFallbackRecoveryProgress::Recovering(
                WindowFallbackRecoveryPhase::MissingLiveMonitor
            )
        );
        {
            let bindings = harness.app.world().resource::<Bindings>();
            assert!(bindings.binding(&first_role).is_err());
            assert!(bindings.binding(&second_role).is_ok());
        }

        harness.advance(2);
        let bindings = harness.app.world().resource::<Bindings>();
        assert!(bindings.role_entity(&first_role).is_err());
        assert!(bindings.role_entity(&second_role).is_ok());

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

    #[test]
    fn explicit_retirement_clears_every_window_recovery_policy_and_baseline() -> Result<(), String>
    {
        let mut harness = RecoveryHarness::new()?;
        for (suffix, recovery_policy) in [
            ("forget", RecoveryPolicy::Forget),
            ("request", RecoveryPolicy::ReapplyOnRequest),
            ("return", RecoveryPolicy::ReapplyOnReturn),
        ] {
            let (role, device) = harness.bind(
                &format!("window:managed:{suffix}"),
                &format!("{suffix}-display"),
                recovery_policy,
            )?;
            harness
                .app
                .world_mut()
                .resource_mut::<WindowFallbackRecoveryState>()
                .mark_missing(role.clone());
            harness
                .app
                .world_mut()
                .resource_mut::<StrandedWindowMovementBaselines>()
                .begin(role.clone());
            harness
                .app
                .world_mut()
                .resource_mut::<StrandedWindowMovementBaselines>()
                .observe_display(&role, &device);

            harness
                .app
                .world_mut()
                .trigger(RetireRole { role: role.clone() });

            assert!(
                harness
                    .app
                    .world()
                    .resource::<Bindings>()
                    .binding(&role)
                    .is_err()
            );
            assert_eq!(
                harness
                    .app
                    .world()
                    .resource::<WindowFallbackRecoveryState>()
                    .phase(&role),
                WindowFallbackRecoveryProgress::NotRecovering
            );
            assert!(
                !harness
                    .app
                    .world()
                    .resource::<StrandedWindowMovementBaselines>()
                    .is_tracked(&role)
            );
            assert!(
                !harness
                    .app
                    .world()
                    .resource::<StrandedWindowMovementBaselines>()
                    .is_tracked(&role)
            );
        }
        Ok(())
    }
}
