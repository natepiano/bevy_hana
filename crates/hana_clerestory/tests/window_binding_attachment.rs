//! Application-owned output windows attach to an existing Clerestory driver binding.

use std::error::Error;

use bevy::prelude::*;
use hana_clerestory::ManagedWindow;
use hana_clerestory::ManagedWindowPersistence;
use hana_clerestory::RequestedWindowPlacement;
use hana_clerestory::WindowManagerPlugin;
use hana_clerestory::WindowPlacementConfiguration;
use hana_clerestory::WindowRoleAttachmentError;
use hana_clerestory::attach_window_to_binding_role;
use hana_clerestory::window_placement_driver;
use hana_rigging::prelude::*;

const fn display(identity: u64) -> DeviceKey {
    DeviceKey {
        kind: DeviceKind::Display,
        id:   DeviceIdSource::Synthesized {
            digest: Digest::new(identity),
        },
    }
}

fn authoring(
    app: &App,
    role: RoleKey,
    device: DeviceKey,
) -> Result<BindingAuthoring<WindowPlacementConfiguration>, Box<dyn Error>> {
    let configuration = WindowPlacementConfiguration::Requested(
        RequestedWindowPlacement::fullscreen_for_display(&device)?,
    );
    Ok(BindingAuthoring::new(
        role,
        DeviceEndpoint {
            device,
            id: hana_clerestory::DisplayEndpointPart::Output.endpoint_id()?,
        },
        window_placement_driver(app.world())?,
        configuration,
        BindingPolicy::new(
            RecoveryPolicy::ReapplyOnReturn,
            RetryOn::NewRevision,
            OnAbort::LeaveAsIs,
            OnSessionLoss::Recreate,
            ApplyDeadline::ProcessDefault,
        ),
    ))
}

#[test]
fn attachment_refuses_missing_role_and_missing_window_without_registering_a_role()
-> Result<(), Box<dyn Error>> {
    let mut app = App::new();
    let state_directory = tempfile::tempdir()?;
    app.add_plugins(WindowManagerPlugin::with_path(
        state_directory.path().join("windows.ron"),
    ));
    let role = RoleKey::new("hana.output.attachment-refusal")?;
    let window = app.world_mut().spawn(Window::default()).id();
    assert!(matches!(
        attach_window_to_binding_role(app.world_mut(), window, &role),
        Err(WindowRoleAttachmentError::RoleNotRegistered(_))
    ));
    assert!(app.world().resource::<Bindings>().binding(&role).is_err());
    let binding = authoring(&app, role.clone(), display(6900))?;
    register_binding(app.world_mut(), binding)?;
    let not_window = app.world_mut().spawn_empty().id();
    assert!(matches!(
        attach_window_to_binding_role(app.world_mut(), not_window, &role),
        Err(WindowRoleAttachmentError::WindowMissing)
    ));
    app.world_mut().despawn(window);
    assert!(matches!(
        attach_window_to_binding_role(app.world_mut(), window, &role),
        Err(WindowRoleAttachmentError::WindowMissing)
    ));
    Ok(())
}

#[test]
fn repeated_attachment_preserves_application_binding_and_never_enables_managed_persistence()
-> Result<(), Box<dyn Error>> {
    let mut app = App::new();
    let state_directory = tempfile::tempdir()?;
    app.add_plugins(WindowManagerPlugin::with_path(
        state_directory.path().join("windows.ron"),
    ));
    let role = RoleKey::new("hana.output.attachment-idempotence")?;
    let device = display(6901);
    let binding = authoring(&app, role.clone(), device.clone())?;
    let role_entity = register_binding(app.world_mut(), binding)?;
    let window = app.world_mut().spawn(Window::default()).id();
    let entity_count = app.world().entities().len();
    for _ in 0..3 {
        attach_window_to_binding_role(app.world_mut(), window, &role)?;
        assert_eq!(
            app.world().resource::<Bindings>().role_entity(&role)?,
            role_entity
        );
        assert_eq!(
            app.world()
                .resource::<Bindings>()
                .binding(&role)?
                .endpoint
                .device,
            device
        );
        assert_eq!(app.world().entities().len(), entity_count);
        assert!(app.world().get::<ManagedWindow>(window).is_none());
        assert!(
            app.world()
                .get::<ManagedWindowPersistence>(window)
                .is_none()
        );
    }
    Ok(())
}

#[test]
fn attachment_resolves_replacement_role_and_window_despawn_preserves_logical_binding()
-> Result<(), Box<dyn Error>> {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::window::WindowPlugin {
            primary_window: None,
            exit_condition: bevy::window::ExitCondition::DontExit,
            ..default()
        },
    ));
    app.init_resource::<bevy::winit::WinitMonitors>();
    let state_directory = tempfile::tempdir()?;
    app.add_plugins(WindowManagerPlugin::with_path(
        state_directory.path().join("windows.ron"),
    ));
    let role = RoleKey::new("hana.output.attachment-replacement")?;
    let first = authoring(&app, role.clone(), display(6902))?;
    let original = register_binding(app.world_mut(), first)?;
    app.update();
    app.update();
    let window = app.world_mut().spawn(Window::default()).id();
    attach_window_to_binding_role(app.world_mut(), window, &role)?;
    let replacement = authoring(&app, role.clone(), display(6903))?;
    let current = replace_binding(app.world_mut(), replacement)?;
    attach_window_to_binding_role(app.world_mut(), window, &role)?;
    app.world_mut().despawn(current);
    app.update();
    app.update();
    let current = app.world().resource::<Bindings>().role_entity(&role)?;
    assert_ne!(
        original, current,
        "kernel recreates the missing role entity"
    );
    attach_window_to_binding_role(app.world_mut(), window, &role)?;
    assert_eq!(
        app.world().resource::<Bindings>().role_entity(&role)?,
        current
    );
    assert!(app.world().get::<Window>(window).is_some());
    app.world_mut().despawn(window);
    app.world_mut().flush();
    assert_eq!(
        app.world().resource::<Bindings>().role_entity(&role)?,
        current,
        "a disposable window cannot own or retire the logical output binding"
    );
    let reopened = app.world_mut().spawn(Window::default()).id();
    attach_window_to_binding_role(app.world_mut(), reopened, &role)?;
    assert!(app.world().get::<ManagedWindow>(reopened).is_none());
    Ok(())
}

#[test]
fn fullscreen_request_is_authored_without_a_window_and_rejects_other_device_kinds()
-> Result<(), Box<dyn Error>> {
    let device = display(6904);
    let requested = RequestedWindowPlacement::fullscreen_for_display(&device)?;
    assert!(requested.logical_size.x > 0 && requested.logical_size.y > 0);
    let configuration = WindowPlacementConfiguration::Requested(requested);
    assert!(matches!(
        configuration,
        WindowPlacementConfiguration::Requested(_)
    ));
    let camera = DeviceKey {
        kind: DeviceKind::Camera,
        ..device
    };
    assert!(RequestedWindowPlacement::fullscreen_for_display(&camera).is_err());
    Ok(())
}
