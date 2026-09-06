use std::time::Duration;

use bevy::ecs::component::Component;
use bevy::ecs::entity::Entity;
use bevy::ecs::entity::EntityHashMap;
use bevy::ecs::event::EntityEvent;
use bevy::ecs::query::Added;
use bevy::ecs::query::Changed;
use bevy::ecs::query::Has;
use bevy::ecs::query::Or;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::reflect::ReflectEvent;
use bevy::ecs::system::Commands;
use bevy::ecs::system::Local;
use bevy::ecs::system::Query;
use bevy::ecs::system::SystemParam;
use bevy::reflect::Reflect;

use super::easing::SequenceEasing;
use super::playback::SequenceCommandOutcome;
use super::playback::SequenceDirection;
use super::playback::SequenceMovement;
use super::playback::SequencePlayback;
use super::playback::SequencePlaybackError;
use super::playback::SequencePosition;
use super::stages::SequenceScope;
use super::stages::SequenceStages;
use super::time::SequenceTime;
use super::traversal::SequenceUpdate;

/// Whether a producer sampled its own source state during the current update.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
enum SourceSampling {
    /// The producer has not published a current sample since the last clear.
    #[default]
    Stale,
    /// The producer sampled its own current absolute source state.
    Current,
}

/// Producer-published evidence that its own source state is current.
///
/// Restoration reads this marker and nothing else; no adjacent movement sample
/// stands in for it. A producer marks itself in
/// [`SequencePlaybackSystems::ProduceMovement`](super::SequencePlaybackSystems::ProduceMovement);
/// driver arbitration clears every marker after resolving claims and releases,
/// so a restoration decision reads the most recent production.
#[derive(Component, Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
#[reflect(Component)]
pub struct SequenceSourceState(SourceSampling);

impl SequenceSourceState {
    /// Records that the producer sampled its own current absolute source state.
    pub const fn mark_current(&mut self) { self.0 = SourceSampling::Current; }

    pub(super) const fn is_current(self) -> bool { matches!(self.0, SourceSampling::Current) }

    const fn clear(&mut self) { self.0 = SourceSampling::Stale; }
}

/// The at-most-one valid producer an explicit takeover displaced.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum DisplacedDriver {
    /// No producer was selected when the claim succeeded.
    NoPriorDriver,
    /// This producer was displaced and may be restored when the selection ends.
    Retained(Entity),
}

/// What happened to a target when its selected producer released it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum DriverRestoration {
    /// The displaced producer still targets this sequence, marked its source
    /// current, and resolves its scope, so it is selected again.
    Restored(Entity),
    /// No displaced producer was retained, so the target became unowned.
    NoDisplacedDriver,
    /// The displaced producer no longer targets this sequence, was not current,
    /// or no longer resolves its scope, so the target became unowned.
    DisplacedDriverStale(Entity),
}

/// Who owns one sequence's local position.
///
/// The same vocabulary names the issuer of a [`SequenceCommand`], so a producer
/// commands on its own behalf and ordinary domain playback commands as
/// [`SequenceOwner::NativePlayback`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum SequenceOwner {
    /// No producer is selected, so native commands own local position.
    NativePlayback,
    /// This producer is selected and is the only permitted position writer.
    Driver(Entity),
}

/// Whether a domain has retained playback to own at all.
///
/// [`SequenceOwner::NativePlayback`] means an existing retained sequence is
/// available for native commands. It must not be used to describe an entity
/// whose domain playback has not been prepared yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum SequenceOwnership {
    /// The domain has no retained playback on this entity.
    NoRetainedSequence,
    /// The retained sequence exists and this owner controls its position.
    Retained(SequenceOwner),
}

/// Shared targeted playback command vocabulary.
///
/// The shared layer resolves absolute play destinations and common journey
/// state. Each domain resolves adjacent step destinations from its own authored
/// boundary ledger.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum SequenceCommand {
    /// Play forward to the sequence end.
    Play,
    /// Play backward to the sequence start.
    PlayBackward,
    /// Pause an active journey and retain its destination.
    Pause,
    /// Resume a paused journey.
    Resume,
    /// Stop at the current local position.
    Cancel,
    /// Travel to the next authored boundary.
    Step,
    /// Travel to the previous authored boundary.
    StepBackward,
}

/// Result of issuing a [`SequenceCommand`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceCommandResponse {
    /// Arbitration permitted the command and local playback reported this
    /// outcome.
    Permitted(SequenceCommandOutcome),
    /// The issuer does not own local position, so nothing mutated and nothing
    /// queued.
    Rejected(SequenceOwner),
    /// The commanded entity carries no retained playback, so there was no local
    /// position to own or to mutate. This is distinct from
    /// [`Self::Rejected`], which names an owner that does hold one.
    NoRetainedSequence,
}

/// Result of seeking local position directly to an absolute destination.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SequenceSeekResponse {
    /// The seek applied and produced this traversal.
    Sought(SequenceUpdate),
    /// A producer holds the target, so nothing mutated.
    Rejected(SequenceOwner),
}

/// Result of applying the selected producer's movement to local position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SequenceMovementApplication {
    /// No producer is selected, so native playback owns local position.
    NativePlayback,
    /// The selected producer published no movement, so the prior position and
    /// domain output hold.
    HoldingPriorPosition,
    /// The selected producer's movement applied to local position.
    Moved(SequenceUpdate),
    /// The movement contradicted the current local position, so nothing
    /// mutated.
    Invalid(SequencePlaybackError),
}

/// Which stages a producer evaluates and how an external curve relates to
/// authored stage easing.
///
/// Every [`SequenceDriver`] carries this component through Bevy's required
/// components, and its [`Default`] is [`SequenceEvaluation::AUTHORED_WHOLE`],
/// so whole-sequence authored easing is a value present on the entity, never a
/// missing component read as a default.
#[derive(Component, Clone, Debug, PartialEq, Reflect)]
#[reflect(Component)]
pub struct SequenceEvaluation {
    scope:  SequenceScope,
    easing: SequenceEasing,
}

impl SequenceEvaluation {
    /// Whole-sequence scope with only domain-authored stage easing.
    pub const AUTHORED_WHOLE: Self = Self {
        scope:  SequenceScope::WholeSequence,
        easing: SequenceEasing::Authored,
    };

    /// Creates an evaluation from a stage selection and an easing relationship.
    #[must_use]
    pub const fn new(scope: SequenceScope, easing: SequenceEasing) -> Self {
        Self { scope, easing }
    }

    /// Returns the selected stages.
    #[must_use]
    pub const fn scope(&self) -> SequenceScope { self.scope }

    /// Returns how an external curve relates to authored stage easing.
    #[must_use]
    pub const fn easing(&self) -> &SequenceEasing { &self.easing }
}

impl Default for SequenceEvaluation {
    fn default() -> Self { Self::AUTHORED_WHOLE }
}

/// Marks a producer entity as driving one target sequence.
///
/// Inserting or changing this component requests ownership without replacing a
/// producer that already holds the target. Removing it releases the target.
#[derive(Component, Clone, Copy, Debug, Eq, PartialEq, Reflect)]
#[require(SequenceEvaluation, SequenceSourceState)]
#[reflect(Component)]
pub struct SequenceDriver {
    target: Entity,
}

impl SequenceDriver {
    /// Drives the sequence on `target`.
    #[must_use]
    pub const fn new(target: Entity) -> Self { Self { target } }

    /// Returns the driven sequence entity.
    #[must_use]
    pub const fn target(&self) -> Entity { self.target }
}

/// Requests replacement of the target's currently selected producer.
///
/// Insert this beside [`SequenceDriver`] for the one explicit takeover
/// operation. Arbitration records the displaced producer, selects the
/// requester, and removes this component.
#[derive(Component, Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
#[reflect(Component)]
pub struct SequenceDriverTakeover;

/// Triggered on a target when a producer becomes its selected driver.
#[derive(EntityEvent, Clone, Copy, Debug, Reflect)]
#[reflect(Event)]
pub struct SequenceDriverSelected {
    /// Driven sequence entity.
    pub entity:    Entity,
    /// Newly selected producer.
    pub driver:    Entity,
    /// Producer this selection displaced, if any.
    pub displaced: DisplacedDriver,
}

/// Triggered on a target when an ordinary claim did not replace its driver.
#[derive(EntityEvent, Clone, Copy, Debug, Reflect)]
#[reflect(Event)]
pub struct SequenceDriverClaimRejected {
    /// Driven sequence entity.
    pub entity:   Entity,
    /// Producer whose claim was rejected.
    pub driver:   Entity,
    /// Producer that keeps the selection.
    pub selected: Entity,
}

/// Triggered on a target when its selected driver released it.
#[derive(EntityEvent, Clone, Copy, Debug, Reflect)]
#[reflect(Event)]
pub struct SequenceDriverReleased {
    /// Driven sequence entity.
    pub entity:      Entity,
    /// Producer that no longer drives the target.
    pub released:    Entity,
    /// Whether a displaced producer was restored.
    pub restoration: DriverRestoration,
}

/// Triggered on a target when another producer's selection rejected a command.
#[derive(EntityEvent, Clone, Copy, Debug, Reflect)]
#[reflect(Event)]
pub struct SequenceCommandRejected {
    /// Driven sequence entity.
    pub entity:  Entity,
    /// Command that did not mutate the sequence.
    pub command: SequenceCommand,
    /// Producer that currently holds the target.
    pub driver:  Entity,
}

/// The private per-target selection maintained by driver arbitration.
#[derive(Component, Clone, Copy, Debug)]
pub(super) struct SelectedDriver {
    driver:    Entity,
    displaced: DisplacedDriver,
}

/// Applies shared commands, native advancement, and selected-producer movement
/// to one domain's local [`SequencePlayback`].
///
/// Every position-mutating method on [`SequencePlayback`] is crate-private, so
/// this parameter is the only external path to local position. Which issuer may
/// write is cooperative: each method takes the issuer as an argument and
/// arbitrates against the target's selection, rejecting a mismatched claim
/// rather than proving who called.
#[derive(SystemParam)]
pub struct SequenceCommands<'w, 's> {
    commands:   Commands<'w, 's>,
    selections: Query<'w, 's, &'static SelectedDriver>,
    movements:  Query<'w, 's, &'static SequenceMovement>,
}

impl SequenceCommands<'_, '_> {
    /// Returns who currently owns `target`'s local position.
    #[must_use]
    pub fn owner(&self, target: Entity) -> SequenceOwner {
        self.selections
            .get(target)
            .map_or(SequenceOwner::NativePlayback, |selected| {
                SequenceOwner::Driver(selected.driver)
            })
    }

    /// Issues one shared command against `target`.
    ///
    /// A command issued while another producer holds the target is rejected
    /// without mutation or queuing, and triggers [`SequenceCommandRejected`].
    pub fn apply(
        &mut self,
        target: Entity,
        issuer: SequenceOwner,
        playback: &mut SequencePlayback,
        command: SequenceCommand,
    ) -> SequenceCommandResponse {
        let owner = self.owner(target);
        if let SequenceOwner::Driver(selected) = owner
            && issuer != SequenceOwner::Driver(selected)
        {
            self.commands.trigger(SequenceCommandRejected {
                entity: target,
                command,
                driver: selected,
            });
            return SequenceCommandResponse::Rejected(owner);
        }

        let outcome = match command {
            SequenceCommand::Play => playback.play(SequenceDirection::Forward),
            SequenceCommand::PlayBackward => playback.play(SequenceDirection::Backward),
            SequenceCommand::Pause => playback.pause(),
            SequenceCommand::Resume => playback.resume(),
            SequenceCommand::Cancel => playback.cancel(),
            SequenceCommand::Step => playback.step(SequenceDirection::Forward),
            SequenceCommand::StepBackward => playback.step(SequenceDirection::Backward),
        };
        SequenceCommandResponse::Permitted(outcome)
    }

    /// Begins native travel to an absolute interior destination.
    ///
    /// `driver` names the producer issuing the command. Identity is cooperative:
    /// a caller supplying the selected producer's entity is admitted, so this
    /// rejects a mismatched claim rather than proving the caller's identity.
    /// [`SequenceCommands::apply`] arbitrates the same way.
    pub fn play_to(
        &mut self,
        target: Entity,
        driver: Entity,
        playback: &mut SequencePlayback,
        destination: SequencePosition,
    ) -> SequenceCommandResponse {
        let owner = self.owner(target);
        if owner != SequenceOwner::Driver(driver) {
            if let SequenceOwner::Driver(selected) = owner {
                self.commands.trigger(SequenceCommandRejected {
                    entity:  target,
                    command: SequenceCommand::Play,
                    driver:  selected,
                });
            }
            return SequenceCommandResponse::Rejected(owner);
        }
        SequenceCommandResponse::Permitted(playback.play_to(destination))
    }

    /// Advances an active native journey from a domain-supplied `delta`.
    ///
    /// A target held by a producer keeps its position and domain output.
    pub fn advance_native(
        &mut self,
        target: Entity,
        playback: &mut SequencePlayback,
        delta: Duration,
        total: SequenceTime,
    ) -> SequenceUpdate {
        match self.owner(target) {
            SequenceOwner::NativePlayback => playback.advance_native(delta, total),
            SequenceOwner::Driver(_) => SequenceUpdate::NoTraversal,
        }
    }

    /// Moves local position directly to an absolute destination.
    ///
    /// Domains use this for authored jumps that traverse no journey. A target
    /// held by another producer rejects the seek without mutation and triggers
    /// [`SequenceCommandRejected`], so a blocked jump is observable.
    ///
    /// # Errors
    ///
    /// Returns the exact [`SequencePlaybackError`] when `direction` contradicts
    /// the movement from the current position, leaving playback unchanged.
    pub fn try_seek(
        &mut self,
        target: Entity,
        issuer: SequenceOwner,
        playback: &mut SequencePlayback,
        destination: SequencePosition,
        direction: SequenceDirection,
    ) -> Result<SequenceSeekResponse, SequencePlaybackError> {
        let owner = self.owner(target);
        if let SequenceOwner::Driver(selected) = owner
            && issuer != owner
        {
            self.commands.trigger(SequenceCommandRejected {
                entity:  target,
                command: match direction {
                    SequenceDirection::Forward => SequenceCommand::Play,
                    SequenceDirection::Backward => SequenceCommand::PlayBackward,
                },
                driver:  selected,
            });
            return Ok(SequenceSeekResponse::Rejected(owner));
        }
        playback
            .try_seek(f64::from(destination.normalized()), direction)
            .map(SequenceSeekResponse::Sought)
    }

    /// Applies the selected producer's current movement to local position.
    pub fn apply_selected_movement(
        &mut self,
        target: Entity,
        playback: &mut SequencePlayback,
    ) -> SequenceMovementApplication {
        let SequenceOwner::Driver(driver) = self.owner(target) else {
            return SequenceMovementApplication::NativePlayback;
        };
        let Ok(movement) = self.movements.get(driver) else {
            return SequenceMovementApplication::HoldingPriorPosition;
        };
        playback.apply_movement(movement).map_or_else(
            SequenceMovementApplication::Invalid,
            SequenceMovementApplication::Moved,
        )
    }
}

/// Selects one driver per target from the claims raised this update.
///
/// Takeover claims resolve in a first pass over the query and ordinary claims in
/// a second, so the outcome does not depend on the order the query yields:
///
/// - A takeover ends up selected whether the driver it displaces was selected in an earlier update
///   or claimed in this one.
/// - A takeover displacing an already-selected driver reports [`DisplacedDriver::Retained`] of that
///   driver.
/// - A takeover racing an ordinary first claim on a target that had no selection reports
///   [`DisplacedDriver::NoPriorDriver`], because nothing was ever selected; the ordinary claim
///   receives [`SequenceDriverClaimRejected`].
/// - Two takeovers in the same update resolve in query order among themselves; the last one
///   selected reports [`DisplacedDriver::Retained`] of the one before it.
pub(super) fn resolve_driver_claims(
    mut commands: Commands,
    claims: Query<
        (Entity, &SequenceDriver, Has<SequenceDriverTakeover>),
        Or<(Changed<SequenceDriver>, Added<SequenceDriverTakeover>)>,
    >,
    mut selections: Query<&mut SelectedDriver>,
    mut claimed_this_update: Local<EntityHashMap<Entity>>,
) {
    claimed_this_update.clear();
    for resolving_takeovers in [true, false] {
        for (driver, sequence_driver, takeover_requested) in &claims {
            if takeover_requested != resolving_takeovers {
                continue;
            }
            let target = sequence_driver.target();
            if takeover_requested {
                commands.entity(driver).remove::<SequenceDriverTakeover>();
            }

            if let Ok(mut selected) = selections.get_mut(target) {
                if selected.driver == driver {
                    continue;
                }
                if takeover_requested {
                    *selected = SelectedDriver {
                        driver,
                        displaced: DisplacedDriver::Retained(selected.driver),
                    };
                    commands.trigger(SequenceDriverSelected {
                        entity: target,
                        driver,
                        displaced: DisplacedDriver::Retained(selected.driver),
                    });
                } else {
                    commands.trigger(SequenceDriverClaimRejected {
                        entity: target,
                        driver,
                        selected: selected.driver,
                    });
                }
                continue;
            }

            // A selection made earlier in this same update is still queued in
            // `Commands`, so `selections` cannot see it yet; `claimed_this_update`
            // carries it, and a later insert replaces the queued one.
            if let Some(selected) = claimed_this_update.get(&target).copied() {
                if selected != driver {
                    if takeover_requested {
                        let displaced = DisplacedDriver::Retained(selected);
                        claimed_this_update.insert(target, driver);
                        commands
                            .entity(target)
                            .insert(SelectedDriver { driver, displaced });
                        commands.trigger(SequenceDriverSelected {
                            entity: target,
                            driver,
                            displaced,
                        });
                    } else {
                        commands.trigger(SequenceDriverClaimRejected {
                            entity: target,
                            driver,
                            selected,
                        });
                    }
                }
                continue;
            }

            claimed_this_update.insert(target, driver);
            commands.entity(target).insert(SelectedDriver {
                driver,
                displaced: DisplacedDriver::NoPriorDriver,
            });
            commands.trigger(SequenceDriverSelected {
                entity: target,
                driver,
                displaced: DisplacedDriver::NoPriorDriver,
            });
        }
    }
}

pub(super) fn resolve_driver_releases(
    mut commands: Commands,
    selections: Query<(Entity, &SelectedDriver, Option<&SequenceStages>)>,
    drivers: Query<(&SequenceDriver, &SequenceEvaluation, &SequenceSourceState)>,
) {
    for (target, selected, sequence_stages) in &selections {
        if drivers
            .get(selected.driver)
            .is_ok_and(|(sequence_driver, ..)| sequence_driver.target() == target)
        {
            continue;
        }

        let restoration = match selected.displaced {
            DisplacedDriver::NoPriorDriver => DriverRestoration::NoDisplacedDriver,
            DisplacedDriver::Retained(displaced) => {
                if restorable(target, displaced, sequence_stages, &drivers) {
                    DriverRestoration::Restored(displaced)
                } else {
                    DriverRestoration::DisplacedDriverStale(displaced)
                }
            },
        };

        match restoration {
            DriverRestoration::Restored(displaced) => {
                commands.entity(target).insert(SelectedDriver {
                    driver:    displaced,
                    displaced: DisplacedDriver::NoPriorDriver,
                });
            },
            DriverRestoration::NoDisplacedDriver | DriverRestoration::DisplacedDriverStale(_) => {
                commands.entity(target).remove::<SelectedDriver>();
            },
        }
        commands.trigger(SequenceDriverReleased {
            entity: target,
            released: selected.driver,
            restoration,
        });
    }
}

pub(super) fn clear_producer_source_state(mut source_states: Query<&mut SequenceSourceState>) {
    for mut source_state in &mut source_states {
        if source_state.is_current() {
            source_state.clear();
        }
    }
}

fn restorable(
    target: Entity,
    displaced: Entity,
    sequence_stages: Option<&SequenceStages>,
    drivers: &Query<(&SequenceDriver, &SequenceEvaluation, &SequenceSourceState)>,
) -> bool {
    let Ok((sequence_driver, evaluation, source_state)) = drivers.get(displaced) else {
        return false;
    };
    if sequence_driver.target() != target || !source_state.is_current() {
        return false;
    }
    sequence_stages.map_or_else(
        || evaluation.scope() == SequenceScope::WholeSequence,
        |sequence_stages| sequence_stages.resolve(evaluation.scope()).is_ok(),
    )
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::app::App;
    use bevy::app::Update;
    use bevy::ecs::observer::On;
    use bevy::ecs::query::Without;
    use bevy::ecs::resource::Resource;
    use bevy::ecs::schedule::IntoScheduleConfigs;
    use bevy::ecs::system::In;
    use bevy::ecs::system::ResMut;

    use super::*;
    use crate::sequence::SequencePlaybackPlugin;
    use crate::sequence::SequencePlaybackSystems;
    use crate::sequence::traversal::RangeCrossings;

    const MIDDLE_BOUNDARY: f64 = 0.5;
    const MIDDLE_POSITION: f32 = 0.5;

    #[derive(Component)]
    struct TestSequence(SequencePlayback);

    /// A producer that stops publishing a current source sample while keeping its
    /// [`SequenceDriver`].
    #[derive(Component)]
    struct StaleProducer;

    #[derive(Resource, Default)]
    struct EvaluatedPositions(Vec<f32>);

    #[derive(Resource, Default)]
    struct ObservedRestorations(Vec<DriverRestoration>);

    #[derive(Resource, Default)]
    struct RejectedCommands(Vec<SequenceCommand>);

    #[derive(Resource, Default)]
    struct RejectedClaims(Vec<Entity>);

    fn sequence_app() -> App {
        let mut app = App::new();
        app.add_plugins(SequencePlaybackPlugin)
            .init_resource::<EvaluatedPositions>()
            .init_resource::<ObservedRestorations>()
            .init_resource::<RejectedCommands>()
            .init_resource::<RejectedClaims>()
            .add_observer(record_restoration)
            .add_observer(record_rejected_command)
            .add_observer(record_rejected_claim)
            .add_systems(
                Update,
                (
                    mark_producers_current.in_set(SequencePlaybackSystems::ProduceMovement),
                    apply_selected_movements.in_set(SequencePlaybackSystems::ApplyMovement),
                    record_positions.in_set(SequencePlaybackSystems::EvaluateSequences),
                ),
            );
        app
    }

    fn mark_producers_current(
        mut source_states: Query<&mut SequenceSourceState, Without<StaleProducer>>,
    ) {
        for mut source_state in &mut source_states {
            source_state.mark_current();
        }
    }

    fn record_restoration(
        released: On<SequenceDriverReleased>,
        mut observed_restorations: ResMut<ObservedRestorations>,
    ) {
        observed_restorations.0.push(released.restoration);
    }

    fn record_rejected_command(
        rejected: On<SequenceCommandRejected>,
        mut rejected_commands: ResMut<RejectedCommands>,
    ) {
        rejected_commands.0.push(rejected.command);
    }

    fn record_rejected_claim(
        rejected: On<SequenceDriverClaimRejected>,
        mut rejected_claims: ResMut<RejectedClaims>,
    ) {
        rejected_claims.0.push(rejected.driver);
    }

    fn apply_selected_movements(
        mut sequence_commands: SequenceCommands,
        mut sequences: Query<(Entity, &mut TestSequence)>,
    ) {
        for (target, mut sequence) in &mut sequences {
            sequence_commands.apply_selected_movement(target, &mut sequence.0);
        }
    }

    fn record_positions(
        sequences: Query<&TestSequence>,
        mut evaluated_positions: ResMut<EvaluatedPositions>,
    ) {
        for sequence in &sequences {
            evaluated_positions
                .0
                .push(sequence.0.position().normalized());
        }
    }

    fn middle_movement() -> SequenceMovement {
        SequenceMovement::try_new(
            SequencePosition::try_new(MIDDLE_POSITION).expect("0.5 is a valid position"),
            SequenceDirection::Forward,
            0,
            RangeCrossings::NONE,
        )
        .expect("forward movement without repetitions is valid")
    }

    fn test_sequence() -> TestSequence {
        TestSequence(
            SequencePlayback::try_new([MIDDLE_BOUNDARY]).expect("one interior boundary is valid"),
        )
    }

    #[test]
    fn every_driver_carries_a_default_whole_sequence_evaluation() {
        let mut app = sequence_app();
        let target = app.world_mut().spawn(test_sequence()).id();
        let driver = app.world_mut().spawn(SequenceDriver::new(target)).id();

        assert_eq!(
            app.world().entity(driver).get::<SequenceEvaluation>(),
            Some(&SequenceEvaluation::AUTHORED_WHOLE)
        );
        assert_eq!(
            SequenceEvaluation::default(),
            SequenceEvaluation::AUTHORED_WHOLE
        );
    }

    #[test]
    fn selected_movement_applies_before_evaluation_in_the_same_update() {
        let mut app = sequence_app();
        let target = app.world_mut().spawn(test_sequence()).id();
        app.world_mut()
            .spawn((SequenceDriver::new(target), middle_movement()));

        app.update();

        assert_eq!(
            app.world().resource::<EvaluatedPositions>().0,
            vec![MIDDLE_POSITION]
        );
    }

    #[test]
    fn a_selected_driver_without_a_movement_holds_the_prior_position() {
        let mut app = sequence_app();
        let target = app.world_mut().spawn(test_sequence()).id();
        app.world_mut().spawn(SequenceDriver::new(target));

        app.update();
        app.update();

        assert_eq!(
            app.world().resource::<EvaluatedPositions>().0,
            vec![
                SequencePosition::START.normalized(),
                SequencePosition::START.normalized()
            ]
        );
    }

    #[test]
    fn an_ordinary_second_claim_is_rejected_and_explicit_takeover_replaces_the_driver() {
        let mut app = sequence_app();
        let target = app.world_mut().spawn(test_sequence()).id();
        let first = app.world_mut().spawn(SequenceDriver::new(target)).id();
        app.update();

        let second = app.world_mut().spawn(SequenceDriver::new(target)).id();
        app.update();
        assert_eq!(
            selected_driver(&mut app, target),
            SequenceOwner::Driver(first)
        );

        app.world_mut()
            .entity_mut(second)
            .insert(SequenceDriverTakeover);
        app.update();

        assert_eq!(
            selected_driver(&mut app, target),
            SequenceOwner::Driver(second)
        );
        assert!(
            app.world()
                .entity(second)
                .get::<SequenceDriverTakeover>()
                .is_none()
        );
    }

    #[test]
    fn releasing_a_takeover_restores_a_current_displaced_producer() {
        let mut app = sequence_app();
        let target = app.world_mut().spawn(test_sequence()).id();
        let first = app
            .world_mut()
            .spawn((SequenceDriver::new(target), SequenceSourceState::default()))
            .id();
        app.update();
        let second = app
            .world_mut()
            .spawn((SequenceDriver::new(target), SequenceDriverTakeover))
            .id();
        app.update();
        assert_eq!(
            selected_driver(&mut app, target),
            SequenceOwner::Driver(second)
        );

        app.world_mut()
            .entity_mut(second)
            .remove::<SequenceDriver>();
        app.update();

        assert_eq!(
            selected_driver(&mut app, target),
            SequenceOwner::Driver(first)
        );
    }

    #[test]
    fn releasing_a_takeover_leaves_a_stale_displaced_producer_unowned() {
        let mut app = sequence_app();
        let target = app.world_mut().spawn(test_sequence()).id();
        let first = app.world_mut().spawn(SequenceDriver::new(target)).id();
        app.update();
        let second = app
            .world_mut()
            .spawn((SequenceDriver::new(target), SequenceDriverTakeover))
            .id();
        app.update();

        app.world_mut().entity_mut(first).remove::<SequenceDriver>();
        app.world_mut()
            .entity_mut(second)
            .remove::<SequenceDriver>();
        app.update();

        assert_eq!(
            selected_driver(&mut app, target),
            SequenceOwner::NativePlayback
        );
    }

    #[test]
    fn a_takeover_displaces_a_claim_made_in_the_same_update() {
        let mut app = sequence_app();
        let target = app.world_mut().spawn(test_sequence()).id();
        let first = app.world_mut().spawn(SequenceDriver::new(target)).id();
        let second = app
            .world_mut()
            .spawn((SequenceDriver::new(target), SequenceDriverTakeover))
            .id();

        app.update();

        let selected = app
            .world()
            .entity(target)
            .get::<SelectedDriver>()
            .copied()
            .expect("the target carries a selection");
        assert_eq!(selected.driver, second);
        assert_eq!(
            selected.displaced,
            DisplacedDriver::NoPriorDriver,
            "the takeover resolved before the ordinary claim, so no driver was \
             ever selected for it to displace"
        );
        assert_eq!(
            selected_driver(&mut app, target),
            SequenceOwner::Driver(second)
        );
        assert_eq!(app.world().resource::<RejectedClaims>().0, vec![first]);
    }

    #[test]
    fn releasing_a_takeover_refuses_restoration_when_the_displaced_producer_is_not_current() {
        let mut app = sequence_app();
        let target = app.world_mut().spawn(test_sequence()).id();
        let first = app.world_mut().spawn(SequenceDriver::new(target)).id();
        app.update();
        let second = app
            .world_mut()
            .spawn((SequenceDriver::new(target), SequenceDriverTakeover))
            .id();
        app.update();

        app.world_mut().entity_mut(first).insert(StaleProducer);
        app.update();
        app.world_mut()
            .entity_mut(second)
            .remove::<SequenceDriver>();
        app.update();

        assert_eq!(
            app.world().entity(first).get::<SequenceDriver>(),
            Some(&SequenceDriver::new(target)),
            "the displaced producer still targets the sequence, so restoration \
             reached the freshness check"
        );
        assert!(
            !app.world()
                .entity(first)
                .get::<SequenceSourceState>()
                .copied()
                .expect("every driver carries a source state")
                .is_current()
        );
        assert_eq!(
            last_restoration(&app),
            DriverRestoration::DisplacedDriverStale(first)
        );
        assert_eq!(
            selected_driver(&mut app, target),
            SequenceOwner::NativePlayback
        );
    }

    #[test]
    fn releasing_a_takeover_refuses_restoration_when_the_displaced_scope_no_longer_resolves() {
        let mut app = sequence_app();
        let stages = SequenceStages::new([Duration::from_secs(1), Duration::from_secs(1)]);
        let stage = stages
            .stage_id(0)
            .expect("a two stage description has an ordinal zero");
        let target = app.world_mut().spawn((test_sequence(), stages)).id();
        let first = app
            .world_mut()
            .spawn((
                SequenceDriver::new(target),
                SequenceEvaluation::new(SequenceScope::Stage(stage), SequenceEasing::Authored),
            ))
            .id();
        app.update();
        let second = app
            .world_mut()
            .spawn((SequenceDriver::new(target), SequenceDriverTakeover))
            .id();
        app.update();

        app.world_mut()
            .entity_mut(target)
            .insert(SequenceStages::new([
                Duration::from_secs(1),
                Duration::from_secs(1),
            ]));
        app.world_mut()
            .entity_mut(second)
            .remove::<SequenceDriver>();
        app.update();

        assert!(
            app.world()
                .entity(first)
                .get::<SequenceSourceState>()
                .copied()
                .expect("every driver carries a source state")
                .is_current(),
            "the displaced producer is current, so restoration reached scope \
             revalidation"
        );
        assert_eq!(
            last_restoration(&app),
            DriverRestoration::DisplacedDriverStale(first)
        );
        assert_eq!(
            selected_driver(&mut app, target),
            SequenceOwner::NativePlayback
        );
    }

    #[test]
    fn a_seek_blocked_by_a_producer_triggers_a_command_rejection() {
        let mut app = sequence_app();
        let target = app.world_mut().spawn(test_sequence()).id();
        let driver = app.world_mut().spawn(SequenceDriver::new(target)).id();
        app.update();
        let destination =
            SequencePosition::try_new(MIDDLE_POSITION).expect("0.5 is a valid position");

        let rejected = app
            .world_mut()
            .run_system_cached_with(
                seek_destination,
                (target, SequenceOwner::NativePlayback, destination),
            )
            .expect("the seek system runs");

        assert_eq!(
            rejected,
            Ok(SequenceSeekResponse::Rejected(SequenceOwner::Driver(
                driver
            )))
        );
        assert_eq!(
            app.world().resource::<RejectedCommands>().0,
            vec![SequenceCommand::Play]
        );
    }

    fn seek_destination(
        input: In<(Entity, SequenceOwner, SequencePosition)>,
        mut sequence_commands: SequenceCommands,
        mut sequences: Query<&mut TestSequence>,
    ) -> Result<SequenceSeekResponse, SequencePlaybackError> {
        let (target, issuer, destination) = input.0;
        let mut sequence = sequences
            .get_mut(target)
            .expect("the target carries a test sequence");
        sequence_commands.try_seek(
            target,
            issuer,
            &mut sequence.0,
            destination,
            SequenceDirection::Forward,
        )
    }

    fn last_restoration(app: &App) -> DriverRestoration {
        app.world()
            .resource::<ObservedRestorations>()
            .0
            .last()
            .copied()
            .expect("a release was observed")
    }

    #[test]
    fn native_commands_report_applied_no_change_and_rejection() {
        let mut app = sequence_app();
        let target = app.world_mut().spawn(test_sequence()).id();
        let driver = app.world_mut().spawn(SequenceDriver::new(target)).id();

        let native = issue(
            &mut app,
            target,
            SequenceOwner::NativePlayback,
            SequenceCommand::Play,
        );
        assert_eq!(
            native,
            SequenceCommandResponse::Permitted(SequenceCommandOutcome::Applied)
        );
        let repeated = issue(
            &mut app,
            target,
            SequenceOwner::NativePlayback,
            SequenceCommand::Play,
        );
        assert_eq!(
            repeated,
            SequenceCommandResponse::Permitted(SequenceCommandOutcome::NoChange)
        );

        app.update();

        let rejected = issue(
            &mut app,
            target,
            SequenceOwner::NativePlayback,
            SequenceCommand::Pause,
        );
        assert_eq!(
            rejected,
            SequenceCommandResponse::Rejected(SequenceOwner::Driver(driver))
        );
        let owned = issue(
            &mut app,
            target,
            SequenceOwner::Driver(driver),
            SequenceCommand::Cancel,
        );
        assert_eq!(
            owned,
            SequenceCommandResponse::Permitted(SequenceCommandOutcome::Applied)
        );
    }

    #[test]
    fn owner_authorized_play_to_rejects_a_caller_that_is_not_the_selected_driver() {
        let mut app = sequence_app();
        let target = app.world_mut().spawn(test_sequence()).id();
        let driver = app.world_mut().spawn(SequenceDriver::new(target)).id();
        let impostor = app.world_mut().spawn_empty().id();
        app.update();
        let destination =
            SequencePosition::try_new(MIDDLE_POSITION).expect("0.5 is a valid position");

        let rejected = app
            .world_mut()
            .run_system_cached_with(play_to_destination, (target, impostor, destination))
            .expect("the play-to system runs");
        assert_eq!(
            rejected,
            SequenceCommandResponse::Rejected(SequenceOwner::Driver(driver))
        );

        let permitted = app
            .world_mut()
            .run_system_cached_with(play_to_destination, (target, driver, destination))
            .expect("the play-to system runs");
        assert_eq!(
            permitted,
            SequenceCommandResponse::Permitted(SequenceCommandOutcome::Applied)
        );
    }

    fn play_to_destination(
        input: In<(Entity, Entity, SequencePosition)>,
        mut sequence_commands: SequenceCommands,
        mut sequences: Query<&mut TestSequence>,
    ) -> SequenceCommandResponse {
        let (target, driver, destination) = input.0;
        let mut sequence = sequences
            .get_mut(target)
            .expect("the target carries a test sequence");
        sequence_commands.play_to(target, driver, &mut sequence.0, destination)
    }

    fn issue(
        app: &mut App,
        target: Entity,
        issuer: SequenceOwner,
        command: SequenceCommand,
    ) -> SequenceCommandResponse {
        app.world_mut()
            .run_system_cached_with(issue_command, (target, issuer, command))
            .expect("the command system runs")
    }

    fn issue_command(
        input: In<(Entity, SequenceOwner, SequenceCommand)>,
        mut sequence_commands: SequenceCommands,
        mut sequences: Query<&mut TestSequence>,
    ) -> SequenceCommandResponse {
        let (target, issuer, command) = input.0;
        let mut sequence = sequences
            .get_mut(target)
            .expect("the target carries a test sequence");
        sequence_commands.apply(target, issuer, &mut sequence.0, command)
    }

    fn selected_driver(app: &mut App, target: Entity) -> SequenceOwner {
        app.world_mut()
            .run_system_cached_with(read_owner, target)
            .expect("the owner system runs")
    }

    fn read_owner(target: In<Entity>, sequence_commands: SequenceCommands) -> SequenceOwner {
        sequence_commands.owner(target.0)
    }
}
