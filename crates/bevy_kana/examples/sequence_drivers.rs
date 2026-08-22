//! Shared sequence drivers.
//!
//! Demonstrates the whole shared sequence contract in one headless run:
//!
//! - native commands moving a domain sequence from Bevy time,
//! - a producer claiming the sequence and becoming its only position writer,
//! - authored easing from both a stock Bevy easing and an owned [`EasingCurve`],
//! - an ordinary second claim rejected without replacing the selected producer,
//! - one explicit takeover, then restoration of the displaced producer,
//! - a target with no domain evaluator arbitrating exactly the same way.
//!
//! Run with `cargo run -p bevy_kana --example sequence_drivers`.

use std::error::Error;
use std::time::Duration;

use bevy::MinimalPlugins;
use bevy::app::App;
use bevy::app::Update;
use bevy::ecs::component::Component;
use bevy::ecs::entity::Entity;
use bevy::ecs::observer::On;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::system::In;
use bevy::ecs::system::Query;
use bevy::ecs::system::Res;
use bevy::log::LogPlugin;
use bevy::log::info;
use bevy::math::curve::EaseFunction;
use bevy::time::Time;
use bevy::time::TimeUpdateStrategy;
use bevy_kana::prelude::Easing;
use bevy_kana::prelude::EasingCurve;
use bevy_kana::prelude::EasingCurveError;
use bevy_kana::prelude::EasingInput;
use bevy_kana::prelude::EasingOutput;
use bevy_kana::prelude::SequenceCommand;
use bevy_kana::prelude::SequenceCommandRejected;
use bevy_kana::prelude::SequenceCommandResponse;
use bevy_kana::prelude::SequenceCommands;
use bevy_kana::prelude::SequenceDriver;
use bevy_kana::prelude::SequenceDriverClaimRejected;
use bevy_kana::prelude::SequenceDriverReleased;
use bevy_kana::prelude::SequenceDriverSelected;
use bevy_kana::prelude::SequenceDriverTakeover;
use bevy_kana::prelude::SequenceEasing;
use bevy_kana::prelude::SequenceEasingSample;
use bevy_kana::prelude::SequenceEasingSampler;
use bevy_kana::prelude::SequenceEvaluation;
use bevy_kana::prelude::SequenceMovement;
use bevy_kana::prelude::SequenceMovementApplication;
use bevy_kana::prelude::SequenceOwner;
use bevy_kana::prelude::SequencePlayback;
use bevy_kana::prelude::SequencePlaybackPlugin;
use bevy_kana::prelude::SequencePlaybackSystems;
use bevy_kana::prelude::SequencePosition;
use bevy_kana::prelude::SequenceScope;
use bevy_kana::prelude::SequenceSourceState;
use bevy_kana::prelude::SequenceStages;

/// Fixed step so the printed positions are reproducible.
const DEMO_STEP: Duration = Duration::from_millis(250);
/// Authored stage durations for the demonstration sequence.
const STAGE_DURATIONS: [Duration; 3] = [
    Duration::from_millis(500),
    Duration::from_millis(500),
    Duration::from_secs(1),
];

/// One domain sequence. Domains embed [`SequencePlayback`]; the shared layer
/// never queries it directly.
#[derive(Component)]
struct DemoSequence {
    playback: SequencePlayback,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut app = demo_app();

    let ramp = hold_then_release()?;

    let stages = SequenceStages::new(STAGE_DURATIONS);
    let last_stage = SequenceScope::Stage(stages.stage_id(STAGE_DURATIONS.len() - 1)?);
    let target = app
        .world_mut()
        .spawn((
            DemoSequence {
                playback: SequencePlayback::try_new([0.25, 0.5])?,
            },
            stages,
        ))
        .id();
    let unevaluated_target = app
        .world_mut()
        .spawn(SequenceStages::new(STAGE_DURATIONS))
        .id();
    app.update();

    info!("--- native playback owns the sequence ---");
    command(
        &mut app,
        target,
        SequenceOwner::NativePlayback,
        SequenceCommand::Play,
    );
    app.update();
    app.update();
    command(
        &mut app,
        target,
        SequenceOwner::NativePlayback,
        SequenceCommand::Pause,
    );
    app.update();

    info!("--- a producer claims the sequence and remaps whole-sequence progress ---");
    let remapping_producer = app
        .world_mut()
        .spawn((
            SequenceDriver::new(target),
            SequenceEvaluation::new(
                SequenceScope::WholeSequence,
                SequenceEasing::ComposedWith(Easing::Curve(ramp)),
            ),
            starting_movement(),
            SequenceSourceState::default(),
        ))
        .id();
    app.update();

    info!("--- native commands are rejected while a producer holds the target ---");
    command(
        &mut app,
        target,
        SequenceOwner::NativePlayback,
        SequenceCommand::Play,
    );
    app.update();

    info!("--- an ordinary second claim never displaces the selected producer ---");
    let overshooting_producer = app
        .world_mut()
        .spawn((
            SequenceDriver::new(target),
            SequenceEvaluation::new(
                last_stage,
                SequenceEasing::ReplacedBy(Easing::Bevy(EaseFunction::BackOut)),
            ),
            starting_movement(),
            SequenceSourceState::default(),
        ))
        .id();
    app.update();

    info!("--- one explicit takeover records the displaced producer ---");
    app.world_mut()
        .entity_mut(overshooting_producer)
        .insert(SequenceDriverTakeover);
    app.update();
    app.update();

    info!("--- releasing the takeover restores the displaced producer ---");
    app.world_mut()
        .entity_mut(overshooting_producer)
        .remove::<SequenceDriver>();
    app.update();

    info!("--- a target with no domain evaluator arbitrates identically ---");
    app.world_mut()
        .spawn((SequenceDriver::new(unevaluated_target), starting_movement()));
    app.update();

    info!("--- releasing every producer returns the target to native playback ---");
    app.world_mut()
        .entity_mut(remapping_producer)
        .remove::<SequenceDriver>();
    app.update();
    command(
        &mut app,
        target,
        SequenceOwner::NativePlayback,
        SequenceCommand::Cancel,
    );
    app.update();
    Ok(())
}

fn demo_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(LogPlugin::default())
        .add_plugins(SequencePlaybackPlugin)
        .insert_resource(TimeUpdateStrategy::ManualDuration(DEMO_STEP))
        .add_systems(
            Update,
            (produce_producer_movement, mark_producers_current)
                .chain()
                .in_set(SequencePlaybackSystems::ProduceMovement),
        )
        .add_systems(
            Update,
            (advance_native_journeys, apply_producer_movement)
                .chain()
                .in_set(SequencePlaybackSystems::ApplyMovement),
        )
        .add_systems(
            Update,
            report_sequences.in_set(SequencePlaybackSystems::EvaluateSequences),
        )
        .add_observer(report_selection)
        .add_observer(report_claim_rejection)
        .add_observer(report_release)
        .add_observer(report_command_rejection);
    app
}

/// Authored curve that holds, then releases: the whole-sequence remap stays
/// bounded and monotonic, so the shared layer accepts it.
fn hold_then_release() -> Result<EasingCurve, EasingCurveError> {
    EasingCurve::builder()
        .cubic()
        .knot(EasingInput::try_new(0.0)?, EasingOutput::try_new(0.0)?)
        .knot(EasingInput::try_new(0.6)?, EasingOutput::try_new(0.2)?)
        .knot(EasingInput::try_new(1.0)?, EasingOutput::try_new(1.0)?)
        .try_build()
}

fn starting_movement() -> SequenceMovement { SequenceMovement::default() }

fn command(
    app: &mut App,
    target: Entity,
    issuer: SequenceOwner,
    sequence_command: SequenceCommand,
) {
    let response = app
        .world_mut()
        .run_system_cached_with(issue_command, (target, issuer, sequence_command));
    info!("{sequence_command:?} -> {response:?}");
}

fn issue_command(
    input: In<(Entity, SequenceOwner, SequenceCommand)>,
    mut sequence_commands: SequenceCommands,
    mut sequences: Query<&mut DemoSequence>,
) -> SequenceCommandResponse {
    let (target, issuer, sequence_command) = input.0;
    match sequences.get_mut(target) {
        Ok(mut sequence) => {
            sequence_commands.apply(target, issuer, &mut sequence.playback, sequence_command)
        },
        Err(_) => SequenceCommandResponse::Rejected(sequence_commands.owner(target)),
    }
}

/// Producers advance their own position from their own source state. The shared
/// layer never infers a producer's clock.
fn produce_producer_movement(mut producers: Query<&mut SequenceMovement>, time: Res<Time>) {
    for mut movement in &mut producers {
        let advanced = time
            .delta_secs()
            .mul_add(0.5, movement.position().normalized());
        movement.set_position(
            SequencePosition::try_new(advanced.min(SequencePosition::END.normalized()))
                .unwrap_or(SequencePosition::END),
        );
    }
}

/// A restorable producer publishes its own readiness after sampling its source.
fn mark_producers_current(mut producers: Query<&mut SequenceSourceState>) {
    for mut source_state in &mut producers {
        source_state.mark_current();
    }
}

fn advance_native_journeys(
    mut sequence_commands: SequenceCommands,
    mut sequences: Query<(Entity, &mut DemoSequence, &SequenceStages)>,
    time: Res<Time>,
) {
    for (target, mut sequence, stages) in &mut sequences {
        sequence_commands.advance_native(
            target,
            &mut sequence.playback,
            time.delta(),
            stages.total(),
        );
    }
}

fn apply_producer_movement(
    mut sequence_commands: SequenceCommands,
    mut sequences: Query<(Entity, &mut DemoSequence)>,
) {
    for (target, mut sequence) in &mut sequences {
        let application = sequence_commands.apply_selected_movement(target, &mut sequence.playback);
        if let SequenceMovementApplication::Invalid(error) = application {
            info!("rejected movement for {target}: {error}");
        }
    }
}

fn report_sequences(
    sequence_commands: SequenceCommands,
    sequences: Query<(Entity, &DemoSequence, &SequenceStages)>,
    evaluations: Query<&SequenceEvaluation>,
) {
    let easing_sampler = SequenceEasingSampler;
    for (target, sequence, stages) in &sequences {
        let position = sequence.playback.position();
        let owner = sequence_commands.owner(target);
        let eased = match owner {
            SequenceOwner::NativePlayback => SequenceEasingSample::AuthoredEasingApplies {
                progress: position.normalized(),
            },
            SequenceOwner::Driver(driver) => match evaluations.get(driver) {
                Ok(evaluation) => {
                    let progress = stages
                        .resolve(evaluation.scope())
                        .map_or_else(|_| position.normalized(), |range| range.progress(position));
                    easing_sampler.sample(evaluation.scope(), evaluation.easing(), progress)
                },
                Err(_) => SequenceEasingSample::AuthoredEasingApplies {
                    progress: position.normalized(),
                },
            },
        };
        info!(
            "position {:.3} owner {owner:?} easing {eased:?}",
            position.normalized()
        );
    }
}

fn report_selection(selected: On<SequenceDriverSelected>) {
    info!(
        "selected {} on {} (displaced {:?})",
        selected.driver, selected.entity, selected.displaced
    );
}

fn report_claim_rejection(rejected: On<SequenceDriverClaimRejected>) {
    info!(
        "claim by {} on {} rejected; {} keeps the selection",
        rejected.driver, rejected.entity, rejected.selected
    );
}

fn report_release(released: On<SequenceDriverReleased>) {
    info!(
        "{} released {} ({:?})",
        released.released, released.entity, released.restoration
    );
}

fn report_command_rejection(rejected: On<SequenceCommandRejected>) {
    info!(
        "{:?} on {} rejected; {} holds the target",
        rejected.command, rejected.entity, rejected.driver
    );
}
