use bevy::prelude::Add;
use bevy::prelude::Component;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Time;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy::prelude::Window;
use bevy::prelude::WindowPosition;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::debug;
use bevy::prelude::warn;
use bevy::window::PrimaryWindow;
use bevy_kana::ToU32;
use hana_rigging::prelude::AvailableConfiguration;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::RoleKey;

use crate::Platform;
use crate::constants::EXACT_DISPLAY_WAIT_TIMEOUT_SECS;
use crate::monitors::CurrentMonitor;
use crate::monitors::MonitorDeviceAssociation;
use crate::persistence;
use crate::persistence::EstablishedWindowPlacement;
use crate::persistence::RestorableWindowPosition;
use crate::recovery::StrandedWindowPlacements;
use crate::recovery::WindowFallbackRecoveryState;

/// Prevents `hide_window_on_creation` from hiding a recovery shell.
#[derive(Component)]
pub(crate) struct SkipInitialWindowHide;

/// Hide the primary window when created, before winit creates the OS window.
///
/// Uses an observer on `PrimaryWindow` component addition, so it works regardless
/// of plugin order. The window will be shown after restore completes or immediately
/// if no saved state.
///
/// Note: We observe `Add<PrimaryWindow>` rather than `Add<Window>` because when
/// `Window` is added, `PrimaryWindow` may not exist yet. By observing `PrimaryWindow`,
/// we know the `Window` component already exists on the entity.
pub(crate) fn hide_window_on_creation(
    add: On<Add, PrimaryWindow>,
    mut windows: Query<&mut Window, Without<SkipInitialWindowHide>>,
) {
    debug!(
        "[hide_window_on_creation] Observer fired for entity {:?}",
        add.entity
    );
    if let Ok(mut window) = windows.get_mut(add.entity) {
        debug!("[hide_window_on_creation] Setting window.visible = false");
        window.visible = false;
    }
}

/// Bounded wait for a live display that can satisfy a hidden primary window's saved target.
///
/// The timer advances only while the window is still hidden and no live display can satisfy its
/// role, so it measures time spent waiting for an endpoint rather than time since startup.
#[derive(Resource)]
pub(crate) struct ExactDisplayWait {
    timeout: Timer,
}

impl Default for ExactDisplayWait {
    fn default() -> Self {
        Self {
            timeout: Timer::from_seconds(EXACT_DISPLAY_WAIT_TIMEOUT_SECS, TimerMode::Once),
        }
    }
}

/// Reveal a primary window whose saved display is not plugged in, at the size the user last chose.
///
/// `try_apply_restore` is the only production code that clears the hide applied by
/// [`hide_window_on_creation`], and it runs only once the kernel resolves the role's endpoint to a
/// live display. A saved display that is absent resolves to nothing — whether
/// `author_window_bindings` declined to author a binding at all, or authored one naming that
/// display and left the role waiting on a device that will never arrive. Either way no attempt is
/// ever issued, so without this the window stays hidden for the whole session and the application
/// runs with nothing to click.
///
/// Asking whether a binding exists is therefore the wrong question; the question is whether a live
/// display can satisfy the one this role owns. When none can, the saved geometry is fitted onto the
/// display the window launched on, so the user gets back the window they recognize rather than
/// winit's default rectangle.
///
/// The persisted record and the binding both keep naming the absent display, so
/// `RecoveryPolicy::ReapplyOnReturn` still carries the window home when that display returns.
/// `managed::adopt_live_display_for_stranded_window` is what cancels that return if the user moves
/// the window first.
pub(crate) fn reveal_window_without_exact_display(
    time: Res<Time>,
    bindings: Res<Bindings>,
    association: Res<MonitorDeviceAssociation>,
    platform: Res<Platform>,
    mut wait: ResMut<ExactDisplayWait>,
    mut fallback: ResMut<WindowFallbackRecoveryState>,
    mut stranded: ResMut<StrandedWindowPlacements>,
    mut windows: Query<(&mut Window, Option<&CurrentMonitor>), With<PrimaryWindow>>,
) {
    let Ok((mut window, current_monitor)) = windows.single_mut() else {
        return;
    };
    if window.visible {
        return;
    }
    let Ok(role) = persistence::primary_window_role() else {
        return;
    };
    if a_live_display_can_satisfy(&bindings, &association, &role) {
        return;
    }
    if !wait.timeout.tick(time.delta()).just_finished() {
        return;
    }
    warn!(
        "[reveal_window_without_exact_display] no live display can satisfy the saved target of \
         role {role} after {EXACT_DISPLAY_WAIT_TIMEOUT_SECS}s; revealing the primary window on \
         the display it launched on"
    );
    if let Some(current_monitor) = current_monitor {
        fit_saved_geometry_to_live_monitor(
            &bindings,
            &role,
            current_monitor,
            *platform,
            &mut window,
        );
    }
    fallback.mark_missing(role.clone());
    stranded.begin(role);
    window.visible = true;
}

/// Whether a live display currently answers to the display key this role's binding names.
fn a_live_display_can_satisfy(
    bindings: &Bindings,
    association: &MonitorDeviceAssociation,
    role: &RoleKey,
) -> bool {
    bindings.binding(role).is_ok_and(|binding| {
        association
            .live_descriptor(&binding.endpoint.device)
            .is_some()
    })
}

/// Put the window at its saved size and offset, drawn back inside the display it launched on.
///
/// Nothing here is persisted and the role's authored configuration is untouched: this is only
/// where the window sits until its own display returns. Window mode is left alone, so a saved
/// fullscreen record is shown windowed rather than driven through the fullscreen restore machine
/// on a display it was never sized for.
fn fit_saved_geometry_to_live_monitor(
    bindings: &Bindings,
    role: &RoleKey,
    current_monitor: &CurrentMonitor,
    platform: Platform,
    window: &mut Window,
) {
    let Ok(configuration) = bindings.configuration_for(role) else {
        return;
    };
    let (AvailableConfiguration::LastKnownGood(configuration)
    | AvailableConfiguration::Requested(configuration)) = configuration;
    let Some(placement) = configuration
        .as_any()
        .downcast_ref::<EstablishedWindowPlacement>()
    else {
        return;
    };
    let monitor = &current_monitor.descriptor;
    let fitted = placement.fitted_to(monitor);
    window.resolution.set_physical_resolution(
        (f64::from(fitted.logical_size.x) * monitor.scale).to_u32(),
        (f64::from(fitted.logical_size.y) * monitor.scale).to_u32(),
    );
    if !platform.position_available() {
        return;
    }
    if let RestorableWindowPosition::Restorable {
        physical_position, ..
    } = fitted.restorable_position(monitor)
    {
        window.position = WindowPosition::At(physical_position);
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use bevy::prelude::App;
    use bevy::prelude::IVec2;
    use bevy::prelude::Reflect;
    use bevy::prelude::Time;
    use bevy::prelude::UVec2;
    use bevy::prelude::Update;
    use bevy::prelude::Window;
    use bevy::prelude::With;
    use bevy::prelude::default;
    use bevy::window::PrimaryWindow;
    use hana_rigging::prelude::ApplyDeadline;
    use hana_rigging::prelude::AuthoredId;
    use hana_rigging::prelude::Binding;
    use hana_rigging::prelude::Bindings;
    use hana_rigging::prelude::DeviceEndpoint;
    use hana_rigging::prelude::DeviceIdSource;
    use hana_rigging::prelude::DeviceKey;
    use hana_rigging::prelude::DeviceKind;
    use hana_rigging::prelude::EndpointId;
    use hana_rigging::prelude::LastKnownGoodConfiguration;
    use hana_rigging::prelude::OnAbort;
    use hana_rigging::prelude::OnSessionLoss;
    use hana_rigging::prelude::RecoveryPolicy;
    use hana_rigging::prelude::RequestedConfiguration;
    use hana_rigging::prelude::RetryOn;
    use hana_rigging::prelude::RiggingAppExt;
    use hana_rigging::prelude::RiggingPlugin;
    use hana_rigging::prelude::RoleState;

    use super::ExactDisplayWait;
    use super::reveal_window_without_exact_display;
    use crate::Platform;
    use crate::constants::EXACT_DISPLAY_WAIT_TIMEOUT_SECS;
    use crate::driver::WindowDriverAttemptResults;
    use crate::driver::WindowEndpointDriver;
    use crate::monitors::MonitorDescriptor;
    use crate::monitors::MonitorDeviceAssociation;
    use crate::persistence;
    use crate::recovery::StrandedWindowPlacements;
    use crate::recovery::WindowFallbackRecoveryPhase;
    use crate::recovery::WindowFallbackRecoveryState;

    /// Fraction of the wait that must leave a hidden window hidden.
    const PARTIAL_WAIT_FRACTION: f32 = 0.5;

    /// Geometry standing in for the one live display in these tests.
    fn live_descriptor() -> MonitorDescriptor {
        MonitorDescriptor::for_current_enumeration(0, 1.0, IVec2::ZERO, UVec2::new(1_920, 1_080))
    }

    /// Stands in for the placement a bound role requests; the reveal guard reads only the role.
    #[derive(Reflect)]
    struct BoundRoleConfiguration;

    /// An application holding one primary window and no bindings, matching a startup whose saved
    /// display has no live endpoint.
    fn primary_window_app(visible: bool) -> App {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<Bindings>()
            .init_resource::<ExactDisplayWait>()
            .init_resource::<WindowFallbackRecoveryState>()
            .init_resource::<StrandedWindowPlacements>()
            .init_resource::<MonitorDeviceAssociation>()
            .insert_resource(Platform::detect())
            .add_systems(Update, reveal_window_without_exact_display);
        app.world_mut().spawn((
            Window {
                visible,
                ..default()
            },
            PrimaryWindow,
        ));
        app
    }

    fn advance(app: &mut App, seconds: f32) {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(seconds));
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

    fn recovery_phase(app: &App) -> Option<WindowFallbackRecoveryPhase> {
        let role = persistence::primary_window_role().expect("the primary role key is well formed");
        app.world()
            .resource::<WindowFallbackRecoveryState>()
            .phase(&role)
    }

    #[test]
    fn reveals_a_hidden_window_once_the_wait_expires() {
        let mut app = primary_window_app(false);

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        assert!(window_is_visible(&mut app));
        assert_eq!(
            recovery_phase(&app),
            Some(WindowFallbackRecoveryPhase::MissingLiveMonitor)
        );
    }

    #[test]
    fn leaves_a_hidden_window_hidden_before_the_wait_expires() {
        let mut app = primary_window_app(false);

        advance(
            &mut app,
            EXACT_DISPLAY_WAIT_TIMEOUT_SECS * PARTIAL_WAIT_FRACTION,
        );

        assert!(!window_is_visible(&mut app));
        assert_eq!(recovery_phase(&app), None);
    }

    /// Whether the display a bound role names is currently plugged in.
    #[derive(Clone, Copy)]
    enum BoundDisplay {
        Live,
        Absent,
    }

    /// An application whose primary role already owns a binding, as after a successful authoring.
    fn bound_primary_window_app(bound_display: BoundDisplay) -> Result<App, String> {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin)
            .init_resource::<Time>()
            .init_resource::<ExactDisplayWait>()
            .init_resource::<WindowFallbackRecoveryState>()
            .init_resource::<StrandedWindowPlacements>()
            .init_resource::<WindowDriverAttemptResults>()
            .insert_resource(Platform::detect())
            .add_systems(Update, reveal_window_without_exact_display);
        let driver = app.add_endpoint_driver(WindowEndpointDriver);
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create the primary role: {error}"))?;
        let device_id = AuthoredId::new("reveal-guard-display")
            .map_err(|error| format!("failed to create the test device id: {error}"))?;
        let device = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Authored { value: device_id },
        };
        app.insert_resource(match bound_display {
            BoundDisplay::Live => MonitorDeviceAssociation::from_test_live_displays([(
                device.clone(),
                live_descriptor(),
            )]),
            BoundDisplay::Absent => MonitorDeviceAssociation::default(),
        });
        let binding = Binding {
            role,
            endpoint: DeviceEndpoint {
                device,
                id: EndpointId::Whole,
            },
            driver,
            recovery: RecoveryPolicy::ReapplyOnReturn,
            retry: RetryOn::NewRevision,
            on_abort: OnAbort::default(),
            on_loss: OnSessionLoss::default(),
            state: RoleState::Waiting,
            requested: RequestedConfiguration::new(BoundRoleConfiguration),
            last_known_good: LastKnownGoodConfiguration::NotEstablished,
            apply_deadline: ApplyDeadline::ProcessDefault,
        };
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(binding)
            .map_err(|error| format!("failed to register the primary binding: {error}"))?;
        app.world_mut().spawn((
            Window {
                visible: false,
                ..default()
            },
            PrimaryWindow,
        ));
        Ok(app)
    }

    #[test]
    fn leaves_a_bound_role_whose_display_is_live_to_the_normal_restore() -> Result<(), String> {
        let mut app = bound_primary_window_app(BoundDisplay::Live)?;

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        assert!(!window_is_visible(&mut app));
        assert_eq!(recovery_phase(&app), None);
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
            Some(WindowFallbackRecoveryPhase::MissingLiveMonitor)
        );
        assert!(
            app.world()
                .resource::<StrandedWindowPlacements>()
                .is_tracked(&role)
        );
        Ok(())
    }

    #[test]
    fn records_no_recovery_phase_for_a_window_that_was_never_hidden() {
        let mut app = primary_window_app(true);

        advance(&mut app, EXACT_DISPLAY_WAIT_TIMEOUT_SECS);

        assert!(window_is_visible(&mut app));
        assert_eq!(recovery_phase(&app), None);
    }
}
