//! Retained fold playback: one local sequence position, one evaluator, and the
//! per-member fractions hinge-to-pose conversion reads.

use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::entity::EntityHashMap;
use bevy_ecs::entity::EntityHashSet;
use bevy_ecs::query::Changed;
use bevy_ecs::query::With;
use bevy_ecs::reflect::ReflectComponent;
use bevy_ecs::system::Commands;
use bevy_ecs::system::Query;
use bevy_ecs::system::Res;
use bevy_ecs::system::SystemParam;
use bevy_reflect::Reflect;
use bevy_time::Time;
use bevy_time::Virtual;
use hana_kana::EasingSampler;
use hana_kana::SequenceCommand;
use hana_kana::SequenceCommandResponse;
use hana_kana::SequenceCommands;
use hana_kana::SequenceEasing;
use hana_kana::SequenceEasingError;
use hana_kana::SequenceEasingSample;
use hana_kana::SequenceEasingSampler;
use hana_kana::SequenceEvaluation;
use hana_kana::SequenceMovement;
use hana_kana::SequenceMovementApplication;
use hana_kana::SequenceOwner;
use hana_kana::SequenceOwnership;
use hana_kana::SequencePlayback;
use hana_kana::SequencePlaybackError;
use hana_kana::SequencePosition;
use hana_kana::SequenceRange;
use hana_kana::SequenceScope;
use hana_kana::SequenceUpdate;

use super::EasedFoldFraction;
use super::FoldEvaluationError;
use super::FoldMemberSample;
use super::FoldSequence;
use super::constants::SEQUENCE_END;
use super::constants::SEQUENCE_START;
use super::evaluate;
use super::events;
use crate::Hinge;

/// What one sequence currently holds for one member's eased travel.
///
/// The two non-eased states are distinct because the hinge driver does
/// opposite things with them: an untracked member has no authored travel at all
/// and is posed at its base endpoint, while an unresolved one has authored
/// travel whose fraction this update could not produce and is skipped, so it
/// keeps the pose it already has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FoldMemberFraction {
    /// No retained sequence stages the member, so it rests at its base
    /// endpoint.
    Untracked,
    /// A retained sequence stages the member but has resolved no fraction for
    /// it yet, so its current pose holds.
    Unresolved,
    /// The member's eased travel toward its folded endpoint.
    Eased(EasedFoldFraction),
}

/// Whether a per-driver condition was already reported for the current driver.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum DriverReport {
    #[default]
    Armed,
    Reported(Entity),
}

impl DriverReport {
    /// Returns whether this occurrence should be reported, and records it.
    fn report(&mut self, driver: Entity) -> bool {
        if *self == Self::Reported(driver) {
            return false;
        }
        *self = Self::Reported(driver);
        true
    }

    const fn rearm(&mut self) { *self = Self::Armed; }
}

/// Whether a per-sequence condition was already reported.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum SequenceReport {
    #[default]
    Armed,
    Reported,
}

impl SequenceReport {
    /// Returns whether this occurrence should be reported, and records it.
    pub(super) const fn report(&mut self) -> bool {
        if matches!(*self, Self::Reported) {
            return false;
        }
        *self = Self::Reported;
        true
    }

    const fn rearm(&mut self) { *self = Self::Armed; }
}

/// Retained local playback for one authored [`FoldSequence`].
///
/// The library writes this component; applications read it and command it
/// through [`FoldCommands`], which is the only parameter that can reach the
/// embedded [`SequencePlayback`] this component owns privately. That embedded
/// playback owns local position and native journey state, and the component
/// also holds what the evaluator resolved for each staged member this update.
/// It holds no lease, no writer sample, no clock, and no second easing
/// state.
///
/// Presence of this component is what "prepared runtime state" means: nothing
/// carries a parallel readiness flag. [`crate::FoldPlugin`] inserts it beside
/// every [`FoldSequence`]; [`crate::ArrangementPlugin`] alone does not, which
/// leaves an authored sequence inert by design.
#[derive(Component, Clone, Debug, Reflect)]
#[reflect(opaque)]
#[reflect(Component)]
pub struct FoldSequencePlayback {
    playback:        SequencePlayback,
    fractions:       EntityHashMap<FoldMemberFraction>,
    last_inputs:     LastEvaluatedFoldInputs,
    scope_report:    DriverReport,
    curve_report:    DriverReport,
    movement_report: DriverReport,
    easing_report:   SequenceReport,
    hinge_report:    SequenceReport,
    boundary_report: SequenceReport,
}

impl FoldSequencePlayback {
    /// Returns the current local sequence position.
    #[must_use]
    pub fn position(&self) -> SequencePosition { self.playback.position() }

    /// Returns the current position as raw normalized progress.
    #[must_use]
    pub const fn normalized_position(&self) -> f64 { self.playback.normalized_position() }

    /// Returns the ordered boundary positions of the authored ledger.
    #[must_use]
    pub fn boundary_positions(&self) -> &[f64] { self.playback.boundary_positions() }

    /// Returns the current gap between ordered boundary records.
    #[must_use]
    pub const fn gap(&self) -> usize { self.playback.gap() }

    /// Returns whether a native journey is currently moving.
    #[must_use]
    pub const fn is_playing(&self) -> bool { self.playback.is_playing() }

    /// Returns whether a native journey is paused with its destination retained.
    #[must_use]
    pub const fn is_paused(&self) -> bool { self.playback.is_paused() }

    /// Returns what this sequence currently holds for `member_entity`.
    ///
    /// A member this sequence stages answers
    /// [`FoldMemberFraction::Unresolved`] until evaluation resolves a fraction
    /// for it, and every member it does not stage answers
    /// [`FoldMemberFraction::Untracked`].
    #[must_use]
    pub fn member_fraction(&self, member_entity: Entity) -> FoldMemberFraction {
        self.fractions
            .get(&member_entity)
            .copied()
            .unwrap_or(FoldMemberFraction::Untracked)
    }

    /// Builds idle playback over one authored sequence's boundary ledger.
    ///
    /// Every staged member enters as [`FoldMemberFraction::Unresolved`], so a
    /// hold on the first update after a rebuild is a hold rather than a
    /// silently untracked member resting at its base endpoint.
    fn new(sequence: &FoldSequence) -> Result<Self, FoldPlaybackError> {
        let playback = SequencePlayback::try_new(sequence.ledger().boundary_positions())
            .map_err(FoldPlaybackError)?;

        Ok(Self {
            playback,
            fractions: sequence
                .tracks()
                .iter()
                .map(|track| (track.member_entity(), FoldMemberFraction::Unresolved))
                .collect(),
            last_inputs: LastEvaluatedFoldInputs::default(),
            scope_report: DriverReport::Armed,
            curve_report: DriverReport::Armed,
            movement_report: DriverReport::Armed,
            easing_report: SequenceReport::Armed,
            hinge_report: SequenceReport::Armed,
            boundary_report: SequenceReport::Armed,
        })
    }
}

/// The semantic inputs the movement and evaluation passes last completed.
///
/// Rejected inputs remain distinct from accepted ones so the same invalid
/// state is retried and a later repair can never be mistaken for an idle hold.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
struct LastEvaluatedFoldInputs {
    movement:   LastFoldMovementInputs,
    evaluation: LastFoldEvaluationInputs,
}

#[derive(Clone, Debug, Default, PartialEq, Reflect)]
enum LastFoldMovementInputs {
    #[default]
    AwaitingFirstPass,
    Accepted(FoldMovementInputs),
    Rejected(FoldMovementInputs),
}

#[derive(Clone, Debug, PartialEq, Reflect)]
enum FoldMovementInputs {
    MissingSequence,
    Present {
        position: SequencePosition,
        source:   FoldMovementSource,
    },
}

#[derive(Clone, Debug, PartialEq, Reflect)]
enum FoldMovementSource {
    NativePlayback,
    Driver {
        driver:   Entity,
        movement: DriverMovementInput,
    },
}

#[derive(Clone, Debug, PartialEq, Reflect)]
enum DriverMovementInput {
    Unpublished,
    Published(SequenceMovement),
}

#[derive(Clone, Debug, Default, PartialEq, Reflect)]
enum LastFoldEvaluationInputs {
    #[default]
    AwaitingFirstPass,
    Accepted(FoldEvaluationInputs),
    Rejected(FoldEvaluationInputs),
}

#[derive(Clone, Debug, PartialEq, Reflect)]
enum FoldEvaluationInputs {
    MissingSequence,
    Present {
        position:       SequencePosition,
        source:         FoldEvaluationSource,
        hinged_members: Vec<bool>,
    },
}

#[derive(Clone, Debug, PartialEq, Reflect)]
enum FoldEvaluationSource {
    NativePlayback,
    Driver {
        driver:     Entity,
        evaluation: DriverEvaluationInput,
    },
}

#[derive(Clone, Debug, PartialEq, Reflect)]
enum DriverEvaluationInput {
    Unpublished,
    Published(SequenceEvaluation),
}

/// What every retained fold sequence holds for its staged members this frame.
///
/// This is the batched form of [`FoldSequencePlayback::member_fraction`], with
/// the same states and the same absence rule. The hinge driver keeps
/// one as a system-local scratch value and reloads it each run, so the
/// per-member lookup costs one pass over live sequences and reuses its
/// allocation.
#[derive(Debug, Default)]
pub struct FoldFractionScratch {
    current: EntityHashMap<FoldMemberFraction>,
    prior:   EntityHashMap<FoldMemberFraction>,
    changed: EntityHashSet,
}

impl FoldFractionScratch {
    pub(crate) fn reload(&mut self, playbacks: &Query<&FoldSequencePlayback>) {
        core::mem::swap(&mut self.current, &mut self.prior);
        self.current.clear();
        self.changed.clear();
        for playback in playbacks {
            self.current.extend(
                playback
                    .fractions
                    .iter()
                    .map(|(member_entity, fraction)| (*member_entity, *fraction)),
            );
        }
        self.changed
            .extend(self.current.iter().filter_map(|(&entity, fraction)| {
                (self.prior.get(&entity) != Some(fraction)).then_some(entity)
            }));
        self.changed.extend(
            self.prior
                .keys()
                .filter(|entity| !self.current.contains_key(*entity))
                .copied(),
        );
    }

    /// Answers what [`FoldSequencePlayback::member_fraction`] answers, across
    /// every live sequence at once.
    pub(crate) fn fraction(&self, member_entity: Entity) -> FoldMemberFraction {
        self.current
            .get(&member_entity)
            .copied()
            .unwrap_or(FoldMemberFraction::Untracked)
    }

    /// Iterates the members whose retained fraction differs from the preceding
    /// hinge pass.
    pub(crate) fn changed_members(&self) -> impl Iterator<Item = Entity> + '_ {
        self.changed.iter().copied()
    }
}

/// Issues shared playback commands against retained fold sequences.
///
/// This is the fold domain's application-facing command path: it pairs the
/// shared [`SequenceCommands`] parameter with the retained
/// [`FoldSequencePlayback`] an application cannot mutate itself. The command
/// vocabulary is the shared one; the fold domain adds none of its own.
#[derive(SystemParam)]
pub struct FoldCommands<'w, 's> {
    sequence_commands: SequenceCommands<'w, 's>,
    playbacks:         Query<'w, 's, &'static mut FoldSequencePlayback>,
}

impl FoldCommands<'_, '_> {
    /// Returns who currently owns `arrangement`'s local sequence position.
    #[must_use]
    pub fn owner(&self, arrangement: Entity) -> SequenceOwnership {
        if self.playbacks.get(arrangement).is_err() {
            return SequenceOwnership::NoRetainedSequence;
        }
        SequenceOwnership::Retained(self.sequence_commands.owner(arrangement))
    }

    /// Issues one shared command against `arrangement`'s retained sequence.
    ///
    /// A command issued while another producer holds the sequence is rejected
    /// without mutation or queuing and triggers
    /// [`SequenceCommandRejected`](hana_kana::SequenceCommandRejected). An
    /// entity that carries no retained playback answers
    /// [`SequenceCommandResponse::NoRetainedSequence`] instead, so a caller can
    /// tell a sequence it does not own from one that does not exist.
    pub fn apply(
        &mut self,
        arrangement: Entity,
        issuer: SequenceOwner,
        command: SequenceCommand,
    ) -> SequenceCommandResponse {
        let Ok(mut playback) = self.playbacks.get_mut(arrangement) else {
            return SequenceCommandResponse::NoRetainedSequence;
        };
        self.sequence_commands
            .apply(arrangement, issuer, &mut playback.playback, command)
    }
}

/// The authored ledger produced boundary positions local playback rejected.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FoldPlaybackError(SequencePlaybackError);

/// How the external curve of a selected producer meets authored stage easing
/// for one member.
#[derive(Clone, Copy, Debug)]
enum MemberEasing<'evaluation> {
    /// Authored stage easing applies to the member's own segment progress.
    Authored,
    /// An external curve already ran on the sequence position, so authored
    /// stage easing is suppressed over the selected scope.
    Suppressed,
    /// A one-stage scope: the external curve eases the output of the selected
    /// stage's own segments and leaves every other stage authored.
    OutputCurve {
        stage_ordinal: usize,
        scope:         SequenceScope,
        easing:        &'evaluation SequenceEasing,
    },
}

/// Which authored stage one member is travelling inside at a sampled position.
#[derive(Clone, Copy, Debug, PartialEq)]
enum MemberStage {
    /// The member has no segment at this position, so stage-scoped output
    /// easing never applies to it.
    Unstaged,
    /// The member is inside a segment this ordinal's stage authored.
    Ordinal(usize),
}

impl MemberStage {
    /// Reads which stage, if any, a sampled member is travelling inside.
    const fn of(member_sample: &super::FoldMemberSample<'_>) -> Self {
        match *member_sample {
            FoldMemberSample::Moving { segment, .. } => Self::Ordinal(segment.stage_ordinal()),
            FoldMemberSample::Resting { .. } | FoldMemberSample::Unauthored => Self::Unstaged,
        }
    }
}

/// Which easing path rejected one producer's external curve.
#[derive(Clone, Copy, Debug)]
enum RejectedCurveUse {
    /// The curve was used to ease a multi-stage scope and was rejected there.
    MultiStageScope,
    /// The curve eased the output of one selected stage and produced no
    /// usable value there.
    StageOutputEasing,
}

/// What evaluation resolved for one sequence this update.
#[derive(Clone, Copy, Debug)]
enum EvaluationPlan<'evaluation> {
    /// Hold every current pose without sampling.
    Hold,
    /// Sample every track at this normalized position under this easing.
    Sample {
        position: f64,
        easing:   MemberEasing<'evaluation>,
    },
}

/// Rebuilds retained playback whenever an authored definition is replaced.
///
/// The gate is change detection over [`FoldSequence`] alone.
/// [`FoldSequence`] construction draws a fresh
/// [`SequenceStages`](hana_kana::SequenceStages) revision, so a needless rebuild would strand every
/// producer's definition-bound scope on a stale revision.
///
/// Replacement discards the previous derived state: with no selected driver the
/// native journey is cancelled and the new sequence idles at its start, and
/// with a driver selected the evaluator revalidates that producer's scope
/// against the new description before it evaluates anything. No boundary is
/// crossed between unrelated old and new ledgers.
pub(super) fn rebuild_fold_playback(
    mut commands: Commands,
    sequences: Query<(Entity, &FoldSequence), Changed<FoldSequence>>,
) {
    for (sequence_entity, sequence) in &sequences {
        match FoldSequencePlayback::new(sequence) {
            Ok(playback) => {
                commands
                    .entity(sequence_entity)
                    .insert((playback, sequence.sequence_stages().clone()));
            },
            Err(FoldPlaybackError(error)) => {
                tracing::warn!(target: "hana_valence::diagnostics",
                    sequence = ?sequence_entity,
                    error = ?error,
                    "authored fold ledger produced boundary positions playback rejected",
                );
            },
        }
    }
}

/// Moves each retained sequence's local position exactly once per update, then
/// emits every boundary that movement crossed.
///
/// A sequence with no selected producer advances its own native journey on
/// virtual time. A sequence with one applies that producer's complete movement
/// instead; a producer that published none keeps the prior position and output.
/// Invalid movement mutates nothing and names the arrangement, the producer,
/// the movement, and the playback error.
///
/// Both movement paths return the [`SequenceUpdate`] describing their mutation,
/// and this is the only point at which raw traversal is observable: nothing
/// retains it and evaluation consumes only the new position. Every update that
/// mutated nothing — a hold, a rejected movement, an unchanged position —
/// carries [`SequenceUpdate::NoTraversal`] into
/// [`emit_fold_boundaries`](super::events::emit_fold_boundaries) and emits no
/// event. That function borrows this component's `boundary_report`, so a
/// crossed ordinal the authored ledger cannot answer warns once per sequence
/// instead of dropping its event silently.
pub(super) fn apply_fold_movement(
    mut commands: Commands,
    time: Res<Time<Virtual>>,
    mut sequence_commands: SequenceCommands,
    movements: Query<&SequenceMovement>,
    mut sequences: Query<(Entity, Option<&FoldSequence>, &mut FoldSequencePlayback)>,
) {
    let delta = time.delta();
    for (sequence_entity, sequence, mut retained) in &mut sequences {
        let inputs = fold_movement_inputs(
            sequence_entity,
            sequence,
            &retained,
            &sequence_commands,
            &movements,
        );
        let native_journey_playing = retained.playback.is_playing();
        if !native_journey_playing
            && retained.last_inputs.movement == LastFoldMovementInputs::Accepted(inputs.clone())
        {
            continue;
        }
        let Some(sequence) = sequence else {
            retained.last_inputs.movement = LastFoldMovementInputs::Accepted(inputs);
            continue;
        };
        let total = sequence.total();
        let (update, rejected) = {
            let FoldSequencePlayback {
                playback,
                movement_report,
                ..
            } = &mut *retained;
            match sequence_commands.apply_selected_movement(sequence_entity, playback) {
                SequenceMovementApplication::NativePlayback => {
                    let update =
                        sequence_commands.advance_native(sequence_entity, playback, delta, total);
                    movement_report.rearm();
                    (update, false)
                },
                SequenceMovementApplication::HoldingPriorPosition => {
                    movement_report.rearm();
                    (SequenceUpdate::NoTraversal, false)
                },
                SequenceMovementApplication::Moved(update) => {
                    movement_report.rearm();
                    (update, false)
                },
                SequenceMovementApplication::Invalid(error) => {
                    if let SequenceOwner::Driver(driver) = sequence_commands.owner(sequence_entity)
                        && movement_report.report(driver)
                    {
                        tracing::warn!(target: "hana_valence::diagnostics",
                            arrangement = ?sequence_entity,
                            driver = ?driver,
                            movement = ?movements.get(driver).ok(),
                            error = ?error,
                            "fold movement contradicted local position and was not applied",
                        );
                    }
                    (SequenceUpdate::NoTraversal, true)
                },
            }
        };
        events::emit_fold_boundaries(
            &mut commands,
            sequence_entity,
            sequence,
            update,
            &mut retained.boundary_report,
        );
        let completed_inputs = fold_movement_inputs(
            sequence_entity,
            Some(sequence),
            &retained,
            &sequence_commands,
            &movements,
        );
        retained.last_inputs.movement = if rejected {
            LastFoldMovementInputs::Rejected(completed_inputs)
        } else {
            LastFoldMovementInputs::Accepted(completed_inputs)
        };
    }
}

fn fold_movement_inputs(
    sequence_entity: Entity,
    sequence: Option<&FoldSequence>,
    retained: &FoldSequencePlayback,
    sequence_commands: &SequenceCommands<'_, '_>,
    movements: &Query<&SequenceMovement>,
) -> FoldMovementInputs {
    if sequence.is_none() {
        return FoldMovementInputs::MissingSequence;
    }
    let source = match sequence_commands.owner(sequence_entity) {
        SequenceOwner::NativePlayback => FoldMovementSource::NativePlayback,
        SequenceOwner::Driver(driver) => FoldMovementSource::Driver {
            driver,
            movement: movements.get(driver).copied().map_or(
                DriverMovementInput::Unpublished,
                DriverMovementInput::Published,
            ),
        },
    };
    FoldMovementInputs::Present {
        position: retained.playback.position(),
        source,
    }
}

/// Resolves one eased fraction per staged member from the immutable tracks.
///
/// This is the fold domain's single evaluator. It owns every
/// [`FoldEvaluationError`], holds the current pose in each case rather than
/// substituting a fallback curve, and caches its output so
/// The hinge driver can recompute the pose from the live [`Hinge`] in
/// `PostUpdate`.
pub(super) fn evaluate_fold_sequences(
    sequence_commands: SequenceCommands,
    evaluations: Query<&SequenceEvaluation>,
    hinged: Query<(), With<Hinge>>,
    mut sequences: Query<(Entity, Option<&FoldSequence>, &mut FoldSequencePlayback)>,
) {
    let easing_sampler = EasingSampler;
    let sequence_easing_sampler = SequenceEasingSampler;
    let mut skipped = Vec::new();
    for (sequence_entity, sequence, mut retained) in &mut sequences {
        let inputs = fold_evaluation_inputs(
            sequence_entity,
            sequence,
            &retained,
            &sequence_commands,
            &evaluations,
            &hinged,
        );
        if retained.last_inputs.evaluation == LastFoldEvaluationInputs::Accepted(inputs.clone()) {
            continue;
        }
        let Some(sequence) = sequence else {
            retained.last_inputs.evaluation = LastFoldEvaluationInputs::Accepted(inputs);
            continue;
        };
        let owner = sequence_commands.owner(sequence_entity);
        let plan = resolve_plan(
            sequence,
            &mut retained,
            owner,
            &evaluations,
            sequence_easing_sampler,
            sequence_entity,
        );
        let EvaluationPlan::Sample { position, easing } = plan else {
            retained.last_inputs.evaluation = LastFoldEvaluationInputs::Rejected(inputs);
            continue;
        };

        skipped.clear();
        let mut easing_failed = false;
        let mut curve_rejected = false;
        let mut evaluation_rejected = false;
        for track in sequence.tracks() {
            let member_entity = track.member_entity();
            if !hinged.contains(member_entity) {
                skipped.push(member_entity);
                continue;
            }
            let member_sample = track.sample(position);
            let member_stage = MemberStage::of(&member_sample);
            let evaluated = evaluate::fold_fraction(
                &member_sample,
                |raw_progress| {
                    select_easing(easing, member_stage, raw_progress, sequence_easing_sampler)
                },
                |authored, progress| easing_sampler.sample(authored, progress),
            );
            match evaluated {
                Ok(fraction) => {
                    retained
                        .fractions
                        .insert(member_entity, FoldMemberFraction::Eased(fraction));
                },
                // Interpolating an angle is the only failure that can occur
                // after finite easing has been accepted. It runs in
                // `PostUpdate`, so this member holds its prior pose.
                Err(FoldEvaluationError::UnrepresentableAngle) => {
                    evaluation_rejected = true;
                },
                Err(FoldEvaluationError::ExternalCurveRejected(error)) => {
                    evaluation_rejected = true;
                    curve_rejected = true;
                    report_rejected_curve(
                        sequence_entity,
                        owner,
                        error,
                        RejectedCurveUse::StageOutputEasing,
                        &mut retained,
                    );
                },
                Err(FoldEvaluationError::NonFiniteEasing) => {
                    evaluation_rejected = true;
                    easing_failed = true;
                    if retained.easing_report.report() {
                        tracing::warn!(target: "hana_valence::diagnostics",
                            arrangement = ?sequence_entity,
                            member = ?member_entity,
                            "fold easing produced a non-finite output; the pose holds",
                        );
                    }
                },
            }
        }
        // Both conditions warn once per sequence, so a sibling member that
        // sampled cleanly must not rearm them.
        if !easing_failed {
            retained.easing_report.rearm();
        }
        if !curve_rejected {
            retained.curve_report.rearm();
        }
        report_hinge_less_tracks(sequence_entity, &skipped, &mut retained);
        retained.last_inputs.evaluation = if evaluation_rejected {
            LastFoldEvaluationInputs::Rejected(inputs)
        } else {
            LastFoldEvaluationInputs::Accepted(inputs)
        };
    }
}

fn fold_evaluation_inputs(
    sequence_entity: Entity,
    sequence: Option<&FoldSequence>,
    retained: &FoldSequencePlayback,
    sequence_commands: &SequenceCommands<'_, '_>,
    evaluations: &Query<&SequenceEvaluation>,
    hinged: &Query<(), With<Hinge>>,
) -> FoldEvaluationInputs {
    let Some(sequence) = sequence else {
        return FoldEvaluationInputs::MissingSequence;
    };
    let source = match sequence_commands.owner(sequence_entity) {
        SequenceOwner::NativePlayback => FoldEvaluationSource::NativePlayback,
        SequenceOwner::Driver(driver) => FoldEvaluationSource::Driver {
            driver,
            evaluation: evaluations.get(driver).cloned().map_or(
                DriverEvaluationInput::Unpublished,
                DriverEvaluationInput::Published,
            ),
        },
    };
    FoldEvaluationInputs::Present {
        position: retained.playback.position(),
        source,
        hinged_members: sequence
            .tracks()
            .iter()
            .map(|track| hinged.contains(track.member_entity()))
            .collect(),
    }
}

/// Resolves where to sample and how an external curve meets authored easing.
fn resolve_plan<'evaluation>(
    sequence: &FoldSequence,
    retained: &mut FoldSequencePlayback,
    owner: SequenceOwner,
    evaluations: &'evaluation Query<&SequenceEvaluation>,
    sequence_easing_sampler: SequenceEasingSampler,
    sequence_entity: Entity,
) -> EvaluationPlan<'evaluation> {
    let raw_position = retained.playback.normalized_position();
    let SequenceOwner::Driver(driver) = owner else {
        return EvaluationPlan::Sample {
            position: raw_position,
            easing:   MemberEasing::Authored,
        };
    };
    let Ok(evaluation) = evaluations.get(driver) else {
        return EvaluationPlan::Sample {
            position: raw_position,
            easing:   MemberEasing::Authored,
        };
    };
    let scope = evaluation.scope();
    let range = match sequence.sequence_stages().resolve(scope) {
        Ok(range) => {
            retained.scope_report.rearm();
            range
        },
        Err(error) => {
            if retained.scope_report.report(driver) {
                tracing::warn!(target: "hana_valence::diagnostics",
                    arrangement = ?sequence_entity,
                    driver = ?driver,
                    error = ?error,
                    "fold producer scope no longer resolves against the authored stages; \
                     the pose holds",
                );
            }
            return EvaluationPlan::Hold;
        },
    };
    match (evaluation.easing(), scope) {
        (SequenceEasing::Authored, _) => EvaluationPlan::Sample {
            position: raw_position,
            easing:   MemberEasing::Authored,
        },
        (easing, SequenceScope::Stage(stage_id)) => EvaluationPlan::Sample {
            position: raw_position,
            easing:   MemberEasing::OutputCurve {
                stage_ordinal: stage_id.ordinal(),
                scope,
                easing,
            },
        },
        (easing, SequenceScope::WholeSequence | SequenceScope::StageRange { .. }) => {
            let progress = range.progress(retained.playback.position());
            match sequence_easing_sampler.sample(scope, easing, progress) {
                SequenceEasingSample::AuthoredEasingApplies { progress } => {
                    EvaluationPlan::Sample {
                        position: remapped(range, progress),
                        easing:   MemberEasing::Authored,
                    }
                },
                SequenceEasingSample::AuthoredEasingSuppressed { eased } => {
                    EvaluationPlan::Sample {
                        position: remapped(range, eased),
                        easing:   MemberEasing::Suppressed,
                    }
                },
                SequenceEasingSample::CurveRejected(error) => {
                    report_rejected_curve(
                        sequence_entity,
                        owner,
                        error,
                        RejectedCurveUse::MultiStageScope,
                        retained,
                    );
                    EvaluationPlan::Hold
                },
            }
        },
    }
}

/// Reports one producer's unusable curve once, naming the path that rejected it.
fn report_rejected_curve(
    sequence_entity: Entity,
    owner: SequenceOwner,
    error: SequenceEasingError,
    rejected_use: RejectedCurveUse,
    retained: &mut FoldSequencePlayback,
) {
    let SequenceOwner::Driver(driver) = owner else {
        return;
    };
    if !retained.curve_report.report(driver) {
        return;
    }
    match rejected_use {
        RejectedCurveUse::MultiStageScope => tracing::warn!(target: "hana_valence::diagnostics",
            arrangement = ?sequence_entity,
            driver = ?driver,
            error = ?error,
            "fold producer curve was rejected for its multi-stage scope and the pose holds; \
             a single-stage scope accepts the same curve as output easing",
        ),
        RejectedCurveUse::StageOutputEasing => tracing::warn!(target: "hana_valence::diagnostics",
            arrangement = ?sequence_entity,
            driver = ?driver,
            error = ?error,
            "fold producer curve was rejected as output easing for its selected stage and the \
             pose holds",
        ),
    }
}

/// Reports the staged members that carry no [`Hinge`] once per sequence.
fn report_hinge_less_tracks(
    sequence_entity: Entity,
    skipped: &[Entity],
    retained: &mut FoldSequencePlayback,
) {
    if skipped.is_empty() {
        retained.hinge_report.rearm();
        return;
    }
    if retained.hinge_report.report() {
        tracing::warn!(target: "hana_valence::diagnostics",
            arrangement = ?sequence_entity,
            members = ?skipped,
            "fold sequence stages members that carry no Hinge; they were skipped",
        );
    }
}

/// Answers the authored-easing decision for one member's segment progress.
fn select_easing(
    easing: MemberEasing<'_>,
    member_stage: MemberStage,
    raw_progress: f32,
    sequence_easing_sampler: SequenceEasingSampler,
) -> SequenceEasingSample {
    match easing {
        MemberEasing::Authored => SequenceEasingSample::AuthoredEasingApplies {
            progress: raw_progress,
        },
        MemberEasing::Suppressed => SequenceEasingSample::AuthoredEasingSuppressed {
            eased: raw_progress,
        },
        MemberEasing::OutputCurve {
            stage_ordinal: selected,
            scope,
            easing,
        } => {
            if member_stage == MemberStage::Ordinal(selected) {
                sequence_easing_sampler.sample(scope, easing, raw_progress)
            } else {
                SequenceEasingSample::AuthoredEasingApplies {
                    progress: raw_progress,
                }
            }
        },
    }
}

/// Places scope-relative progress back on the whole-sequence position axis.
fn remapped(range: SequenceRange, progress: f32) -> f64 {
    let start = f64::from(range.start().normalized());
    let width = f64::from(range.end().normalized()) - start;

    start
        .mul_add(1.0, f64::from(progress) * width)
        .clamp(SEQUENCE_START, SEQUENCE_END)
}
