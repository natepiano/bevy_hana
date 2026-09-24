//! A refused caller transaction can undo only its still-pending replacement.

use std::error::Error;
use std::num::NonZeroUsize;
use std::time::Duration;

use bevy::prelude::*;
use hana_rigging::prelude::*;
use hana_rigging_scripted::ScriptedDriver;

#[derive(Component, Clone, Debug, PartialEq, Eq, Reflect)]
struct OutputConfiguration(u32);

fn app() -> (App, EndpointDriverRegistration<OutputConfiguration>) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, RiggingPlugin));
    let (driver, _) = ScriptedDriver::<OutputConfiguration>::new();
    let driver = app.add_endpoint_driver(driver);
    (app, driver)
}

const fn endpoint(identity: u64) -> DeviceEndpoint {
    DeviceEndpoint {
        device: DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Synthesized {
                digest: Digest::new(identity),
            },
        },
        id:     EndpointId::Whole,
    }
}

fn authoring(
    driver: EndpointDriverRegistration<OutputConfiguration>,
    role: &RoleKey,
    identity: u64,
    configuration: u32,
) -> BindingAuthoring<OutputConfiguration> {
    BindingAuthoring::new(
        role.clone(),
        endpoint(identity),
        driver,
        OutputConfiguration(configuration),
        BindingPolicy::new(
            RecoveryPolicy::ReapplyOnReturn,
            RetryOn::Interval(Duration::from_secs(1)),
            OnAbort::LeaveAsIs,
            OnSessionLoss::Recreate,
            ApplyDeadline::Authored(Duration::from_secs(5)),
        ),
    )
}

#[test]
fn saturated_replacement_cancellation_restores_authoring_and_endpoint_ownership()
-> Result<(), Box<dyn Error>> {
    let (mut app, driver) = app();
    let role = RoleKey::new("output/original")?;
    let previous = register_binding(app.world_mut(), authoring(driver, &role, 1, 11))?;
    app.update();
    app.world_mut()
        .resource_mut::<Bindings>()
        .set_pending_transition_capacity(NonZeroUsize::MIN)?;
    let replacement = replace_binding(app.world_mut(), authoring(driver, &role, 2, 22))?;
    assert_eq!(previous, replacement);
    assert!(
        app.world_mut()
            .resource_mut::<Bindings>()
            .retire(&role)
            .is_err()
    );

    cancel_pending_binding_replacement(app.world_mut(), &role)?;

    let bindings = app.world().resource::<Bindings>();
    assert_eq!(bindings.binding(&role)?.endpoint, endpoint(1));
    assert_eq!(bindings.role_entity(&role)?, previous);
    assert_eq!(
        bindings.roles_for(&endpoint(1).device).collect::<Vec<_>>(),
        vec![&role]
    );
    assert_eq!(bindings.roles_for(&endpoint(2).device).count(), 0);
    assert_configuration(&app, &role, 11)?;
    assert!(app.world().get_entity(replacement).is_ok());
    let competitor = RoleKey::new("output/competitor")?;
    assert!(matches!(
        register_binding(app.world_mut(), authoring(driver, &competitor, 1, 33)),
        Err(BindingError::EndpointAlreadyOwned { owner, .. }) if owner == role
    ));
    register_binding(app.world_mut(), authoring(driver, &competitor, 2, 33))?;
    Ok(())
}

#[test]
fn replacement_cancellation_preserves_older_same_role_and_unrelated_pending_work()
-> Result<(), Box<dyn Error>> {
    let (mut app, driver) = app();
    app.world_mut()
        .resource_mut::<Bindings>()
        .set_pending_transition_capacity(NonZeroUsize::new(4).ok_or("capacity")?)?;
    let role = RoleKey::new("output/replaced")?;
    let other = RoleKey::new("output/other")?;
    let original = register_binding(app.world_mut(), authoring(driver, &role, 1, 11))?;
    let middle = replace_binding(app.world_mut(), authoring(driver, &role, 2, 22))?;
    let other_entity = register_binding(app.world_mut(), authoring(driver, &other, 4, 44))?;
    let latest = replace_binding(app.world_mut(), authoring(driver, &role, 3, 33))?;

    cancel_pending_binding_replacement(app.world_mut(), &role)?;
    assert_eq!(
        app.world().resource::<Bindings>().role_entity(&role)?,
        middle
    );
    assert_eq!(
        app.world().resource::<Bindings>().binding(&role)?.endpoint,
        endpoint(2)
    );
    assert_eq!(latest, original);
    assert!(app.world().get_entity(latest).is_ok());
    cancel_pending_binding_replacement(app.world_mut(), &role)?;
    assert_eq!(
        app.world().resource::<Bindings>().role_entity(&role)?,
        original
    );
    assert_eq!(
        app.world().resource::<Bindings>().binding(&role)?.endpoint,
        endpoint(1)
    );
    assert_eq!(
        app.world().resource::<Bindings>().role_entity(&other)?,
        other_entity
    );
    assert_eq!(middle, original);
    assert!(app.world().get_entity(middle).is_ok());
    app.update();
    assert_eq!(
        app.world().resource::<Bindings>().role_entity(&role)?,
        original
    );
    assert_eq!(
        app.world().resource::<Bindings>().role_entity(&other)?,
        other_entity
    );
    assert!(app.world().get_entity(original).is_ok());
    assert!(app.world().get_entity(other_entity).is_ok());
    Ok(())
}

#[test]
fn replacement_cancellation_refuses_after_transition_drain_without_mutation()
-> Result<(), Box<dyn Error>> {
    let (mut app, driver) = app();
    let role = RoleKey::new("output/drained")?;
    register_binding(app.world_mut(), authoring(driver, &role, 1, 11))?;
    app.update();
    let replacement = replace_binding(app.world_mut(), authoring(driver, &role, 2, 22))?;
    app.update();

    assert!(cancel_pending_binding_replacement(app.world_mut(), &role).is_err());
    assert_eq!(
        app.world().resource::<Bindings>().role_entity(&role)?,
        replacement
    );
    assert_eq!(
        app.world().resource::<Bindings>().binding(&role)?.endpoint,
        endpoint(2)
    );
    assert_configuration(&app, &role, 22)?;
    Ok(())
}

fn assert_configuration(app: &App, role: &RoleKey, expected: u32) -> Result<(), Box<dyn Error>> {
    let AvailableConfiguration::Requested(configuration) =
        app.world().resource::<Bindings>().configuration_for(role)?
    else {
        return Err("absent devices cannot have an applied configuration".into());
    };
    assert_eq!(
        configuration.downcast_ref::<OutputConfiguration>(),
        Some(&OutputConfiguration(expected))
    );
    Ok(())
}
