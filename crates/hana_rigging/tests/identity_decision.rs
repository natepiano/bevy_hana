//! Noninteractive behavior coverage for the interactive identity-decision example.

use std::error::Error;

use bevy::prelude::App;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::MinimalPlugins;
use bevy::prelude::Update;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleStatus;

#[path = "../examples/identity_decision.rs"]
mod identity_decision_example;

use identity_decision_example::PanelSessionEstablishment;
use identity_decision_example::PanelSessionEstablishmentObservation;
use identity_decision_example::ScriptedRig;
use identity_decision_example::attempt_store_counts;
use identity_decision_example::discard_completed_panel_attempts;
use identity_decision_example::establishment_observation;
use identity_decision_example::finish_converging_applies;
use identity_decision_example::install_saved_panel_kernel;
use identity_decision_example::interactive_example_entry;
use identity_decision_example::retains_lease_and_configuration;

const ESTABLISHMENT_FRAME_CEILING: usize = 8;

#[test]
fn established_panel_retains_lease_and_configuration_together() -> Result<(), Box<dyn Error>> {
    let (mut app, rig) = identity_example_app()?;

    let establishment = advance_until_establishment(&mut app, &rig)?;

    assert_eq!(
        establishment,
        PanelSessionEstablishment::LeaseAndConfigurationRetained
    );
    assert!(retains_lease_and_configuration(app.world(), rig.role()));
    Ok(())
}

#[test]
fn missing_attempt_reports_loss_without_retaining_a_session() -> Result<(), Box<dyn Error>> {
    let (mut app, rig) = identity_example_app()?;
    app.add_systems(
        Update,
        discard_completed_panel_attempts.after(finish_converging_applies),
    );

    let establishment = advance_until_establishment(&mut app, &rig)?;

    assert_eq!(
        establishment,
        PanelSessionEstablishment::MissingAttemptReported
    );
    assert!(!retains_lease_and_configuration(app.world(), rig.role()));
    Ok(())
}

fn identity_example_app() -> Result<(App, ScriptedRig), Box<dyn Error>> {
    std::hint::black_box(interactive_example_entry);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    let rig = install_saved_panel_kernel(&mut app)?;
    Ok((app, rig))
}

fn advance_until_establishment(
    app: &mut App,
    rig: &ScriptedRig,
) -> Result<PanelSessionEstablishment, Box<dyn Error>> {
    for _ in 0..ESTABLISHMENT_FRAME_CEILING {
        app.update();
        match establishment_observation(app.world()) {
            PanelSessionEstablishmentObservation::NotObserved => {},
            PanelSessionEstablishmentObservation::Observed(establishment) => {
                return Ok(establishment);
            },
        }
    }

    Err(format!(
        "the identity-decision driver received no establishment callback; role status: {}; \
         attempt store: {:?}",
        role_status_at_ceiling(app, rig.role()),
        attempt_store_counts(app.world())
    )
    .into())
}

fn role_status_at_ceiling(app: &App, role: &RoleKey) -> String {
    let Ok(role_entity) = app.world().resource::<Bindings>().role_entity(role) else {
        return "binding entity unavailable".to_owned();
    };
    let Ok(role_entity) = app.world().get_entity(role_entity) else {
        return "binding entity despawned".to_owned();
    };
    let Some(role_status) = role_entity.get::<RoleStatus>() else {
        return "RoleStatus unavailable".to_owned();
    };
    format!("{:?}", role_status.view())
}
