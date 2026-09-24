//! Real driver attachment and readback tests, included by the library test build.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]

use bevy::prelude::*;
use bevy::window::ExitCondition;
use bevy::window::OnMonitor;
use bevy::window::WindowMode;
use bevy::window::WindowPlugin;
use hana_rigging::prelude::*;

use crate::CurrentMonitor;
use crate::DisplayTestAdapter;
use crate::DisplayTestDescriptor;
use crate::DisplayTestDeviceKey;
use crate::DisplayTestReporterLookup;
use crate::ManagedWindow;
use crate::ManagedWindowPersistence;
use crate::Monitors;
use crate::Platform;
use crate::ReadbackWindowPosition;
use crate::RequestedWindowPlacement;
use crate::WindowManagerPlugin;
use crate::WindowPlacementConfiguration;
use crate::attach_window_to_binding_role;
use crate::driver::window_attempt_lookup;
use crate::driver::window_session_lookup;
use crate::managed::window_for_role_entity;
use crate::persistence::EstablishedWindowPlacement;
use crate::persistence::EstablishedWindowPosition;
use crate::restore::InjectedWinitWindows;
use crate::restore::WindowRestoreAttempt;
use crate::window_placement_driver;

#[derive(Resource, Default)]
struct RestoreOutcomes {
    successes:  Vec<Entity>,
    mismatches: Vec<Entity>,
}

struct AttachmentFixture {
    app:             App,
    role:            RoleKey,
    device:          DeviceKey,
    monitor:         Entity,
    state_directory: tempfile::TempDir,
}

impl AttachmentFixture {
    fn new() -> Self {
        let state_directory = tempfile::tempdir().expect("isolated window state");
        let displays = DisplayTestAdapter::new();
        displays.observe_displays(vec![DisplayTestDescriptor::new(
            "projector",
            b"attachment-projector".to_vec(),
            PlatformDeviceHandle::Reported(
                ReportedId::new("attachment-projector").expect("display handle"),
            ),
        )]);
        let DisplayTestDeviceKey::Keyed(device) = displays.display_device_key("projector") else {
            panic!("display must have reporter identity");
        };
        let mut app = App::new();
        app.insert_resource(Platform::FIXTURE).add_plugins((
            MinimalPlugins,
            WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            },
            displays.clone(),
            WindowManagerPlugin::with_path(state_directory.path().join("windows.ron")),
        ));
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(100),
        ))
        .init_resource::<RestoreOutcomes>()
        .add_observer(
            |restored: On<crate::WindowRestored>, mut outcomes: ResMut<RestoreOutcomes>| {
                outcomes.successes.push(restored.entity);
            },
        );
        app.add_observer(
            |mismatch: On<crate::WindowRestoreMismatch>, mut outcomes: ResMut<RestoreOutcomes>| {
                outcomes.mismatches.push(mismatch.entity);
            },
        );
        let DisplayTestReporterLookup::Registered(reporter) =
            displays.display_reporter(app.world())
        else {
            panic!("monitor reporter is installed");
        };
        hana_rigging_scripted::advance_reporter(&mut app, reporter)
            .expect("enumerate exact display");
        let monitor = app
            .world()
            .resource::<Monitors>()
            .iter()
            .next()
            .expect("scripted monitor")
            .entity;
        Self {
            app,
            role: RoleKey::new("hana.output.attachment-driver").expect("output role"),
            device,
            monitor,
            state_directory,
        }
    }

    fn authoring(&self) -> BindingAuthoring<WindowPlacementConfiguration> {
        BindingAuthoring::new(
            self.role.clone(),
            DeviceEndpoint {
                device: self.device.clone(),
                id:     crate::DisplayEndpointPart::Output
                    .endpoint_id()
                    .expect("output endpoint"),
            },
            window_placement_driver(self.app.world()).expect("production driver"),
            WindowPlacementConfiguration::Requested(
                RequestedWindowPlacement::fullscreen_for_display(&self.device)
                    .expect("fullscreen request"),
            ),
            BindingPolicy::new(
                RecoveryPolicy::ReapplyOnReturn,
                RetryOn::NewRevision,
                OnAbort::LeaveAsIs,
                OnSessionLoss::Recreate,
                ApplyDeadline::ProcessDefault,
            ),
        )
    }

    fn window(&mut self) -> Entity {
        let descriptor = *self
            .app
            .world()
            .resource::<Monitors>()
            .iter()
            .next()
            .expect("monitor descriptor")
            .descriptor;
        let window = self
            .app
            .world_mut()
            .spawn((
                Window::default(),
                OnMonitor(self.monitor),
                CurrentMonitor {
                    descriptor,
                    effective_window_mode: WindowMode::Windowed,
                },
            ))
            .id();
        self.app.world_mut().init_resource::<InjectedWinitWindows>();
        self.app
            .world_mut()
            .resource_mut::<InjectedWinitWindows>()
            .insert(window, UVec2::ZERO);
        window
    }

    fn advance(&mut self) { self.app.update(); }

    fn assert_successful_restore(&self, window: Entity, operation: &str, successes_before: usize) {
        let outcomes = self.app.world().resource::<RestoreOutcomes>();
        assert!(
            outcomes.mismatches.is_empty(),
            "{operation}: no restore mismatch"
        );
        let successes = &outcomes.successes;
        assert_eq!(
            successes.len(),
            successes_before + 1,
            "{operation}: production settle must succeed"
        );
        assert_eq!(successes.last(), Some(&window));
    }

    fn assert_established_readback(&self, window: Entity) {
        let bindings = self.app.world().resource::<Bindings>();
        let AvailableConfiguration::LastKnownGood(configuration) = bindings
            .configuration_for(&self.role)
            .expect("established binding")
        else {
            panic!("readback must be established");
        };
        let Some(WindowPlacementConfiguration::Established(placement)) =
            configuration.downcast_ref::<WindowPlacementConfiguration>()
        else {
            panic!("successful production settlement must replace Requested with Established");
        };
        assert_eq!(
            placement.saved_window_mode,
            crate::SavedWindowMode::BorderlessFullscreen
        );
        let actual_window = self
            .app
            .world()
            .get::<Window>(window)
            .expect("live window readback");
        let actual_monitor = self
            .app
            .world()
            .get::<CurrentMonitor>(window)
            .expect("live monitor readback");
        assert_eq!(actual_monitor.effective_window_mode, actual_window.mode);
        assert_eq!(
            placement.saved_window_mode,
            crate::SavedWindowMode::from(&actual_monitor.effective_window_mode)
        );
        assert_eq!(
            placement.logical_size.as_vec2(),
            actual_window.resolution.size()
        );
        assert_eq!(
            actual_window.position,
            bevy::window::WindowPosition::At(actual_monitor.descriptor.physical_position)
        );
        assert_eq!(
            placement.position,
            EstablishedWindowPosition::Restorable {
                logical_offset: IVec2::ZERO,
            }
        );
    }

    fn apply_and_read_back(&mut self, window: Entity, operation: &str) {
        for _ in 0..8 {
            self.advance();
            if self
                .app
                .world()
                .get::<WindowRestoreAttempt>(window)
                .is_some()
            {
                break;
            }
        }
        let Some(restoration) = self.app.world().get::<WindowRestoreAttempt>(window) else {
            panic!("{operation}: driver receives attachment");
        };
        let attempt = restoration.attempt();
        assert!(
            matches!(window_attempt_lookup(self.app.world(), &self.role), AttemptLookup::Applying(found) if found == attempt)
        );
        for _ in 0..8 {
            self.advance();
            if self
                .app
                .world()
                .get::<Window>(window)
                .is_some_and(|window| matches!(window.mode, WindowMode::BorderlessFullscreen(_)))
            {
                break;
            }
        }
        let window_state = self
            .app
            .world()
            .get::<Window>(window)
            .expect("window remains alive");
        assert_eq!(
            window_state.mode,
            WindowMode::BorderlessFullscreen(
                self.app
                    .world()
                    .resource::<Monitors>()
                    .iter()
                    .find(|monitor| monitor.entity == self.monitor)
                    .expect("exact bound monitor")
                    .descriptor
                    .index
                    .selection()
            )
        );
        let observed_mode = window_state.mode;
        let descriptor = self
            .app
            .world()
            .get::<CurrentMonitor>(window)
            .expect("observed monitor")
            .descriptor;
        {
            let mut window_state = self
                .app
                .world_mut()
                .get_mut::<Window>(window)
                .expect("platform window observation");
            window_state.position = bevy::window::WindowPosition::At(descriptor.physical_position);
            window_state
                .resolution
                .set_physical_resolution(descriptor.physical_size.x, descriptor.physical_size.y);
        }
        self.app
            .world_mut()
            .get_mut::<CurrentMonitor>(window)
            .expect("platform monitor observation")
            .effective_window_mode = observed_mode;
        let successes_before = self
            .app
            .world()
            .resource::<RestoreOutcomes>()
            .successes
            .len();
        for _ in 0..16 {
            self.advance();
            if matches!(
                window_session_lookup(self.app.world(), &self.role),
                SessionLookup::Holding(_)
            ) {
                break;
            }
        }
        self.assert_successful_restore(window, operation, successes_before);
        assert!(
            self.app
                .world()
                .get::<WindowRestoreAttempt>(window)
                .is_none()
        );
        assert!(matches!(
            window_session_lookup(self.app.world(), &self.role),
            SessionLookup::Holding(_)
        ));
        self.assert_established_readback(window);
    }
}

#[test]
fn real_window_driver_defers_until_attachment_applies_and_reattaches_replaced_role() {
    let mut fixture = AttachmentFixture::new();
    let binding = fixture.authoring();
    let original_role = register_binding(fixture.app.world_mut(), binding).expect("output binding");
    for _ in 0..4 {
        fixture.advance();
    }
    let status = fixture
        .app
        .world()
        .get::<RoleStatus>(original_role)
        .expect("role status");
    assert!(
        matches!(
            status.view(),
            RoleStatusView::Waiting(WaitingStatusView::ApplicationTargetAttachmentRequired { .. })
        ),
        "real driver must defer for application attachment: {:?}",
        status.view()
    );
    assert_eq!(
        window_for_role_entity(fixture.app.world(), original_role),
        None
    );
    let bindings = fixture.app.world().resource::<Bindings>();
    let AvailableConfiguration::Requested(configuration) = bindings
        .configuration_for(&fixture.role)
        .expect("requested configuration")
    else {
        panic!("no readback exists before attachment");
    };
    assert!(matches!(
        configuration.downcast_ref::<WindowPlacementConfiguration>(),
        Some(WindowPlacementConfiguration::Requested(_))
    ));
    let window = fixture.window();
    attach_window_to_binding_role(fixture.app.world_mut(), window, &fixture.role)
        .expect("attach original role");
    assert!(fixture.app.world().get::<ManagedWindow>(window).is_none());
    assert!(
        fixture
            .app
            .world()
            .get::<ManagedWindowPersistence>(window)
            .is_none()
    );
    fixture.apply_and_read_back(window, "initial attachment");
    let binding = fixture.authoring();
    let replacement =
        replace_binding(fixture.app.world_mut(), binding).expect("replacement binding");
    attach_window_to_binding_role(fixture.app.world_mut(), window, &fixture.role)
        .expect("attach replacement role");
    fixture.apply_and_read_back(window, "binding replacement");
    let SessionLookup::Holding(session) = window_session_lookup(fixture.app.world(), &fixture.role)
    else {
        panic!("replacement establishes the window session");
    };
    fixture.app.world_mut().despawn(replacement);
    for _ in 0..4 {
        fixture.advance();
    }
    let replacement = fixture
        .app
        .world()
        .resource::<Bindings>()
        .role_entity(&fixture.role)
        .expect("kernel recreates the removed role");
    assert_ne!(replacement, original_role);
    attach_window_to_binding_role(fixture.app.world_mut(), window, &fixture.role)
        .expect("reattach current role entity");
    assert_eq!(
        window_for_role_entity(fixture.app.world(), replacement),
        Some(window)
    );
    assert!(matches!(
        window_session_lookup(fixture.app.world(), &fixture.role),
        SessionLookup::Holding(current) if current == session
    ));
    fixture.app.world_mut().despawn(window);
    fixture.app.world_mut().flush();
    for _ in 0..4 {
        fixture.advance();
    }
    assert_eq!(
        window_for_role_entity(fixture.app.world(), replacement),
        None
    );
    assert!(
        !matches!(
            window_session_lookup(fixture.app.world(), &fixture.role),
            SessionLookup::Holding(_)
        ),
        "a despawned window cannot remain an established driver session"
    );
    if let Ok(saved) = std::fs::read_to_string(fixture.state_directory.path().join("windows.ron")) {
        assert!(
            !saved.contains(fixture.role.as_str()),
            "application role cannot persist managed placement"
        );
    }
}

#[test]
fn unavailable_readback_and_positionless_platform_both_remain_compositor_controlled() {
    let mut fixture = AttachmentFixture::new();
    let window = fixture.window();
    let state = fixture.app.world().get::<Window>(window).expect("window");
    let current = fixture
        .app
        .world()
        .get::<CurrentMonitor>(window)
        .expect("monitor");
    for (position, platform) in [
        (ReadbackWindowPosition::Unavailable, Platform::X11),
        (
            ReadbackWindowPosition::Observed(IVec2::new(17, 29)),
            Platform::Wayland,
        ),
    ] {
        let readback =
            EstablishedWindowPlacement::from_readback(state, current, position, platform);
        assert_eq!(
            readback.position,
            EstablishedWindowPosition::CompositorControlled
        );
    }
    let observed = EstablishedWindowPlacement::from_readback(
        state,
        current,
        ReadbackWindowPosition::Observed(IVec2::new(17, 29)),
        Platform::X11,
    );
    assert!(matches!(
        observed.position,
        EstablishedWindowPosition::Restorable { .. }
    ));
}
