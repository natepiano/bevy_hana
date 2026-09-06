//! Behavior coverage for operating-system confirmation of Clerestory output windows.

use std::time::Duration;

use bevy::prelude::App;
use bevy::prelude::Entity;
use bevy::prelude::On;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Time;
use bevy::time::Virtual;
use bevy::window::PrimaryWindow;
use hana_rigging::prelude::OutputConfirmation;
use hana_rigging::prelude::OutputConfirmationSource;
use hana_rigging::prelude::OutputLost;
use hana_rigging::prelude::OutputProof;
use hana_rigging::prelude::RecoveryPolicy;

use crate::driver::WindowRoleDriverState;
use crate::managed;
use crate::managed::ManagedWindow;
use crate::managed::WindowBindingAuthoring;
use crate::managed::WindowRiggingRole;
use crate::output_proof::OnScreenConfirmation;
use crate::output_proof::ScriptedWindowOnScreenReadings;
use crate::output_proof::WindowOnScreenReading;
use crate::restore::InjectedWinitWindows;
use crate::visibility::tests;
use crate::visibility::tests::AbandonedPlacementScript;
use crate::visibility::tests::BoundConfigurationHistory;
use crate::visibility::tests::BoundDisplay;
use crate::visibility::tests::BoundWindow;

#[derive(Debug, PartialEq)]
enum OutputProofObservation {
    Present(OutputProof),
    Absent,
}

#[derive(Default, Resource)]
struct RecordedOutputLosses(Vec<(Entity, Duration)>);

fn record_output_loss(lost: On<OutputLost>, mut losses: ResMut<RecordedOutputLosses>) {
    losses.0.push((lost.presenter, lost.since));
}

fn managed_output_window_app() -> Result<(App, Entity), String> {
    let (mut app, window) = tests::bound_window_app(
        BoundDisplay::Live,
        BoundWindow::Managed("output proof window"),
        BoundConfigurationHistory::NeverSaved,
        RecoveryPolicy::ReapplyOnReturn,
    )?;
    app.init_resource::<WindowRoleDriverState>()
        .add_observer(managed::on_managed_window_removed)
        .add_observer(managed::on_primary_window_removed)
        .add_observer(managed::on_window_rigging_role_removed);
    app.world_mut().init_resource::<InjectedWinitWindows>();
    Ok((app, window))
}

fn primary_output_window_app() -> Result<(App, Entity), String> {
    tests::abandoned_placement_app(AbandonedPlacementScript::WantedDisplayDeparted)
}

fn half_cadence_seconds() -> f32 { OnScreenConfirmation.cadence().as_secs_f32() / 2.0 }

fn elapsed(app: &App) -> Duration { app.world().resource::<Time<Virtual>>().elapsed() }

fn observe_output_proof(app: &App, window: Entity) -> OutputProofObservation {
    match app.world().get::<OutputProof>(window) {
        Some(output_proof) => OutputProofObservation::Present(output_proof.clone()),
        None => OutputProofObservation::Absent,
    }
}

fn script(app: &mut App, window: Entity, reading: WindowOnScreenReading) {
    app.world_mut()
        .resource_mut::<ScriptedWindowOnScreenReadings>()
        .script(window, reading);
}

fn confirm_visible_window(app: &mut App, window: Entity) -> Result<Duration, String> {
    script(app, window, WindowOnScreenReading::ReportedVisible);
    tests::advance(app, half_cadence_seconds());
    let confirmed_at = elapsed(app);
    let expected = OutputProof::Confirmed { at: confirmed_at };
    let output_proof_observation = observe_output_proof(app, window);
    if output_proof_observation != OutputProofObservation::Present(expected) {
        return Err(format!(
            "the visible output window did not confirm: {output_proof_observation:?}"
        ));
    }
    Ok(confirmed_at)
}

fn begin_recording_output_losses(app: &mut App) {
    app.init_resource::<RecordedOutputLosses>()
        .add_observer(record_output_loss);
}

fn recorded_output_losses(app: &App) -> &[(Entity, Duration)] {
    &app.world().resource::<RecordedOutputLosses>().0
}

#[test]
fn a_reported_visible_output_window_reads_confirmed() -> Result<(), String> {
    let (mut app, window) = managed_output_window_app()?;

    let confirmed_at = confirm_visible_window(&mut app, window)?;

    assert_eq!(
        observe_output_proof(&app, window),
        OutputProofObservation::Present(OutputProof::Confirmed { at: confirmed_at })
    );
    Ok(())
}

#[test]
fn a_reported_occluded_window_ages_to_unconfirmed_and_emits_output_lost_once() -> Result<(), String>
{
    let (mut app, window) = managed_output_window_app()?;
    begin_recording_output_losses(&mut app);
    let confirmed_at = confirm_visible_window(&mut app, window)?;

    script(&mut app, window, WindowOnScreenReading::ReportedOccluded);
    tests::advance(
        &mut app,
        OnScreenConfirmation.cadence().as_secs_f32() + 0.01,
    );

    assert_eq!(
        observe_output_proof(&app, window),
        OutputProofObservation::Present(OutputProof::Unconfirmed {
            since: confirmed_at,
        })
    );
    assert_eq!(recorded_output_losses(&app), &[(window, confirmed_at)]);

    tests::advance(
        &mut app,
        OnScreenConfirmation.cadence().as_secs_f32() + 0.01,
    );
    assert_eq!(
        observe_output_proof(&app, window),
        OutputProofObservation::Present(OutputProof::Unconfirmed {
            since: confirmed_at,
        })
    );
    assert_eq!(recorded_output_losses(&app), &[(window, confirmed_at)]);

    tests::advance(
        &mut app,
        OnScreenConfirmation.cadence().as_secs_f32() + 0.01,
    );
    assert_eq!(
        observe_output_proof(&app, window),
        OutputProofObservation::Present(OutputProof::Unconfirmed {
            since: confirmed_at,
        })
    );
    assert_eq!(recorded_output_losses(&app), &[(window, confirmed_at)]);
    Ok(())
}

#[test]
fn a_window_reported_on_no_screen_after_confirmation_ages_and_emits_output_lost_once()
-> Result<(), String> {
    let (mut app, window) = managed_output_window_app()?;
    begin_recording_output_losses(&mut app);
    let confirmed_at = confirm_visible_window(&mut app, window)?;

    script(&mut app, window, WindowOnScreenReading::OnNoScreen);
    tests::advance(
        &mut app,
        OnScreenConfirmation.cadence().as_secs_f32() + 0.01,
    );

    assert_eq!(
        observe_output_proof(&app, window),
        OutputProofObservation::Present(OutputProof::Unconfirmed {
            since: confirmed_at,
        })
    );
    assert_eq!(recorded_output_losses(&app), &[(window, confirmed_at)]);

    tests::advance(
        &mut app,
        OnScreenConfirmation.cadence().as_secs_f32() + 0.01,
    );
    assert_eq!(
        observe_output_proof(&app, window),
        OutputProofObservation::Present(OutputProof::Unconfirmed {
            since: confirmed_at,
        })
    );
    assert_eq!(recorded_output_losses(&app), &[(window, confirmed_at)]);

    tests::advance(
        &mut app,
        OnScreenConfirmation.cadence().as_secs_f32() + 0.01,
    );
    assert_eq!(
        observe_output_proof(&app, window),
        OutputProofObservation::Present(OutputProof::Unconfirmed {
            since: confirmed_at,
        })
    );
    assert_eq!(recorded_output_losses(&app), &[(window, confirmed_at)]);
    Ok(())
}

#[test]
fn a_window_first_reported_on_no_screen_stays_unconfirmed_without_output_lost() -> Result<(), String>
{
    let (mut app, window) = managed_output_window_app()?;
    begin_recording_output_losses(&mut app);
    let OutputProofObservation::Present(OutputProof::Unconfirmed { since: inserted_at }) =
        observe_output_proof(&app, window)
    else {
        return Err(String::from(
            "the output window did not begin with an unconfirmed proof",
        ));
    };

    script(&mut app, window, WindowOnScreenReading::OnNoScreen);
    tests::advance(
        &mut app,
        OnScreenConfirmation.cadence().as_secs_f32() + half_cadence_seconds(),
    );

    assert_eq!(
        observe_output_proof(&app, window),
        OutputProofObservation::Present(OutputProof::Unconfirmed { since: inserted_at })
    );
    assert!(recorded_output_losses(&app).is_empty());
    Ok(())
}

#[test]
fn the_poll_reads_the_window_once_per_half_cadence() -> Result<(), String> {
    let (mut app, window) = managed_output_window_app()?;
    script(&mut app, window, WindowOnScreenReading::ReportedVisible);
    let readings_before = app
        .world()
        .resource::<ScriptedWindowOnScreenReadings>()
        .readings_taken();

    tests::advance(&mut app, half_cadence_seconds());
    assert_eq!(
        app.world()
            .resource::<ScriptedWindowOnScreenReadings>()
            .readings_taken(),
        readings_before + 1
    );

    tests::advance(&mut app, half_cadence_seconds() * 0.9);
    assert_eq!(
        app.world()
            .resource::<ScriptedWindowOnScreenReadings>()
            .readings_taken(),
        readings_before + 1
    );

    tests::advance(&mut app, half_cadence_seconds() * 0.2);
    assert_eq!(
        app.world()
            .resource::<ScriptedWindowOnScreenReadings>()
            .readings_taken(),
        readings_before + 2
    );
    Ok(())
}

#[test]
fn the_primary_window_confirms_through_the_os_like_every_output_window() -> Result<(), String> {
    let (mut app, primary_window) = primary_output_window_app()?;

    let confirmed_at = confirm_visible_window(&mut app, primary_window)?;

    assert_eq!(
        observe_output_proof(&app, primary_window),
        OutputProofObservation::Present(OutputProof::Confirmed { at: confirmed_at })
    );
    Ok(())
}

#[test]
fn unmanaging_a_non_primary_managed_window_removes_its_source_and_proof() -> Result<(), String> {
    let (mut app, window) = managed_output_window_app()?;
    assert!(
        app.world()
            .get::<OutputConfirmationSource<OnScreenConfirmation>>(window)
            .is_some()
    );
    assert!(matches!(
        observe_output_proof(&app, window),
        OutputProofObservation::Present(_)
    ));

    app.world_mut().entity_mut(window).remove::<ManagedWindow>();
    app.world_mut().flush();

    assert!(
        app.world()
            .get::<OutputConfirmationSource<OnScreenConfirmation>>(window)
            .is_none()
    );
    assert_eq!(
        observe_output_proof(&app, window),
        OutputProofObservation::Absent
    );
    Ok(())
}

#[test]
fn unmanaging_a_reapply_on_return_window_prevents_role_recovery_from_restoring_output_proof()
-> Result<(), String> {
    let (mut app, window) = managed_output_window_app()?;
    let role_entity = app
        .world()
        .get::<WindowRiggingRole>(window)
        .copied()
        .map(WindowRiggingRole::entity)
        .ok_or_else(|| String::from("the managed window had no rigging role"))?;
    confirm_visible_window(&mut app, window)?;

    app.world_mut().entity_mut(window).remove::<ManagedWindow>();
    app.world_mut().flush();
    assert!(
        app.world_mut().despawn(role_entity),
        "the managed window's role entity disappeared before the recovery trigger"
    );
    app.update();

    assert!(app.world().get::<WindowRiggingRole>(window).is_none());
    assert!(
        app.world()
            .get::<OutputConfirmationSource<OnScreenConfirmation>>(window)
            .is_none()
    );
    assert_eq!(
        observe_output_proof(&app, window),
        OutputProofObservation::Absent
    );
    Ok(())
}

#[test]
fn removing_window_rigging_role_removes_its_source_and_proof() -> Result<(), String> {
    let (mut app, window) = managed_output_window_app()?;
    assert!(
        app.world()
            .get::<OutputConfirmationSource<OnScreenConfirmation>>(window)
            .is_some()
    );
    assert!(matches!(
        observe_output_proof(&app, window),
        OutputProofObservation::Present(_)
    ));

    app.world_mut()
        .entity_mut(window)
        .remove::<WindowRiggingRole>();
    app.world_mut().flush();

    assert!(
        app.world()
            .get::<OutputConfirmationSource<OnScreenConfirmation>>(window)
            .is_none()
    );
    assert_eq!(
        observe_output_proof(&app, window),
        OutputProofObservation::Absent
    );
    Ok(())
}

#[test]
fn the_primary_window_keeps_its_proof_when_its_primary_marker_is_removed() -> Result<(), String> {
    let (mut app, managed_window) = managed_output_window_app()?;
    let standing_role = app
        .world()
        .get::<WindowRiggingRole>(managed_window)
        .copied()
        .ok_or_else(|| {
            "the managed window had no rigging role before becoming primary".to_string()
        })?;
    confirm_visible_window(&mut app, managed_window)?;
    let standing_proof = observe_output_proof(&app, managed_window);
    assert!(matches!(standing_proof, OutputProofObservation::Present(_)));

    app.world_mut()
        .entity_mut(managed_window)
        .insert(PrimaryWindow);
    app.world_mut().flush();
    assert!(app.world().get::<PrimaryWindow>(managed_window).is_some());

    app.world_mut()
        .entity_mut(managed_window)
        .remove::<PrimaryWindow>();
    app.world_mut().flush();

    assert!(app.world().get::<PrimaryWindow>(managed_window).is_none());
    assert!(
        app.world()
            .get::<WindowBindingAuthoring>(managed_window)
            .is_none(),
        "the primary-window removal observer did not run"
    );
    assert!(app.world().get::<ManagedWindow>(managed_window).is_some());
    assert_eq!(
        app.world()
            .get::<WindowRiggingRole>(managed_window)
            .map(|role| role.entity()),
        Some(standing_role.entity())
    );
    assert_eq!(observe_output_proof(&app, managed_window), standing_proof);
    assert!(
        app.world()
            .get::<OutputConfirmationSource<OnScreenConfirmation>>(managed_window)
            .is_some()
    );
    Ok(())
}
