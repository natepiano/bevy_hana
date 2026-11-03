//! Retained camera sequences: their ordered [`CameraMove`] authoring and the
//! immutable prepared state that evaluates captured camera poses.

mod commands;
mod controller_installation;
mod playback;
mod request;
#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod support;

use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
pub use commands::CameraCommands;
pub use commands::CameraPlaybackObservation;
pub(super) use controller_installation::CameraControllerInstallationIdentityAllocator;
pub(super) use controller_installation::FreeFlightControllerInstallation;
pub(super) use controller_installation::OrbitControllerInstallation;
pub(super) use controller_installation::clear_selected_driver_input;
pub(super) use controller_installation::identify_free_flight_controller_installation;
pub(super) use controller_installation::identify_orbit_controller_installation;
pub(super) use controller_installation::initialize_camera_controllers;
use hana_kana::SequenceStageId;
use hana_kana::SequenceStageSpan;
use hana_kana::SequenceStages;
use hana_kana::SequenceTime;
pub use playback::CameraEvaluationError;
pub(crate) use playback::CameraSequencePlayback;
pub(super) use playback::apply_camera_sequence_movement;
pub(super) use playback::evaluate_camera_sequences;
pub(super) use playback::finalize_camera_sequence_transitions;
pub(super) use request::CameraSequenceSystems;
pub(super) use request::PendingCameraRequests;
pub(in crate::animation::sequence) use request::RetainedCameraJourney;
#[cfg(test)]
pub(in crate::animation::sequence) use request::RetainedCameraJourneyOrigin;
pub(super) use request::admit_pending_camera_requests;
pub(super) use request::capture_fit_animation;
pub(super) use request::capture_play_animation;
pub(super) use request::close_discarded_camera_sequence;
pub(super) use request::prepare_direct_camera_sequences;
pub(super) use request::remove_direct_camera_sequence;
use thiserror::Error;

use super::queue::CameraMove;

/// An authored camera sequence: one or more [`CameraMove`] steps in play order.
///
/// A sequence is never empty, so its first and last steps always exist. Move
/// order is stable identity within one value: the [`SequenceStages`] built at
/// construction names each step once, and replacing the authored definition
/// produces a new description with a new revision.
///
/// Reflection is opaque: the stage description is derived from the authored
/// moves at construction, and opaque reflection stops a reflected write from
/// setting it independently of the moves it describes.
#[derive(Component, Clone, Debug, Reflect)]
#[reflect(opaque)]
#[reflect(Component)]
pub struct CameraSequence {
    moves:           Vec<CameraMove>,
    sequence_stages: SequenceStages,
}

impl CameraSequence {
    /// Starts a sequence with its first move.
    #[must_use]
    pub fn new(first_move: CameraMove) -> Self { Self::from_moves(vec![first_move]) }

    /// Appends one move after the moves authored so far.
    #[must_use]
    pub fn then(mut self, next_move: CameraMove) -> Self {
        self.moves.push(next_move);
        Self::from_moves(self.moves)
    }

    /// Collects an iterator of moves into a sequence.
    ///
    /// # Errors
    ///
    /// Returns [`CameraSequenceError::NoMoves`] when the iterator yields
    /// nothing.
    pub fn try_from_moves(
        moves: impl IntoIterator<Item = CameraMove>,
    ) -> Result<Self, CameraSequenceError> {
        let moves: Vec<CameraMove> = moves.into_iter().collect();
        if moves.is_empty() {
            return Err(CameraSequenceError::NoMoves);
        }
        Ok(Self::from_moves(moves))
    }

    /// Returns the authored moves in play order.
    #[must_use]
    pub fn moves(&self) -> &[CameraMove] { &self.moves }

    /// Returns the [`SequenceStages`] description used to resolve
    /// definition-bound sequence scopes.
    ///
    /// The description is built once at construction, so its stage identities
    /// stay stable for the lifetime of this value.
    #[must_use]
    pub const fn sequence_stages(&self) -> &SequenceStages { &self.sequence_stages }

    /// Returns the exact sum of every authored move duration.
    #[must_use]
    pub const fn total(&self) -> SequenceTime { self.sequence_stages.total() }

    /// Returns each move's stage identity paired with its exact authored
    /// extent, in play order.
    pub fn stage_ids_with_spans(
        &self,
    ) -> impl Iterator<Item = (SequenceStageId, SequenceStageSpan)> {
        self.sequence_stages.stage_ids_with_spans()
    }

    /// Describes the authored moves, drawing one fresh description revision.
    fn from_moves(moves: Vec<CameraMove>) -> Self {
        let sequence_stages = SequenceStages::new(moves.iter().map(CameraMove::duration));
        Self {
            moves,
            sequence_stages,
        }
    }
}

/// Two sequences are equal when they author the same moves in the same order.
///
/// Stage identity is excluded deliberately: [`SequenceStages`] carries a
/// process-global revision drawn fresh at every construction, so comparing it
/// would make two sequences authored from identical moves never compare equal.
impl PartialEq for CameraSequence {
    fn eq(&self, other: &Self) -> bool { self.moves == other.moves }
}

/// Invalid explicit input supplied to [`CameraSequence`] construction.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum CameraSequenceError {
    /// A sequence was collected from an iterator that yielded no move.
    #[error("a camera sequence must contain at least one move")]
    NoMoves,
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::time::Duration;

    use bevy::curve::easing::EaseFunction;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::prelude::Entity;
    use bevy::reflect::ReflectRef;
    use bevy::reflect::TypeInfo;
    use bevy::reflect::serde::ReflectSerializer;
    use hana_kana::SequenceDirection;
    use hana_kana::SequenceEasingError;
    use hana_kana::SequenceOwner;
    use hana_kana::SequencePosition;

    use super::support::*;
    use super::*;
    use crate::AnimationConflictPolicy;
    use crate::AnimationReason;
    use crate::AnimationRejected;
    use crate::AnimationRejectionReason;
    use crate::AnimationSource;
    use crate::CameraEventTiming;
    use crate::CameraMoveError;
    use crate::CameraRequestPreparationError;
    use crate::PlayAnimation;
    use crate::ZoomContext;
    use crate::ZoomReason;
    use crate::animation::events::AnimationBegin;
    use crate::animation::events::AnimationEnd;
    use crate::animation::events::CameraMoveBegin;
    use crate::animation::events::CameraMoveEnd;
    use crate::fit::ZoomBegin;
    use crate::fit::ZoomEnd;

    #[test]
    fn phase22_camera_event_contracts_auto_register_with_exact_type_paths() -> TestResult {
        let app = camera_sequence_test_app();
        let registry = app.world().resource::<AppTypeRegistry>().read();

        assert_reflected_camera_event::<PlayAnimation>(
            &registry,
            "hana_lagrange::animation::events::PlayAnimation",
        )?;
        assert_reflected_camera_event::<AnimationBegin>(
            &registry,
            "hana_lagrange::animation::events::AnimationBegin",
        )?;
        assert_reflected_camera_event::<AnimationEnd>(
            &registry,
            "hana_lagrange::animation::events::AnimationEnd",
        )?;
        assert_reflected_camera_event::<AnimationRejected>(
            &registry,
            "hana_lagrange::animation::events::AnimationRejected",
        )?;
        assert_reflected_camera_event::<CameraMoveBegin>(
            &registry,
            "hana_lagrange::animation::events::CameraMoveBegin",
        )?;
        assert_reflected_camera_event::<CameraMoveEnd>(
            &registry,
            "hana_lagrange::animation::events::CameraMoveEnd",
        )?;
        assert_reflected_camera_event::<ZoomBegin>(
            &registry,
            "hana_lagrange::fit::triggers::zoom::ZoomBegin",
        )?;
        assert_reflected_camera_event::<ZoomEnd>(
            &registry,
            "hana_lagrange::fit::triggers::zoom::ZoomEnd",
        )?;

        assert_reflected_camera_payload::<AnimationReason>(
            &registry,
            "hana_lagrange::animation::events::AnimationReason",
        )?;
        assert_reflected_camera_payload::<AnimationSource>(
            &registry,
            "hana_lagrange::animation::events::AnimationSource",
        )?;
        assert_reflected_camera_payload::<AnimationRejectionReason>(
            &registry,
            "hana_lagrange::animation::events::AnimationRejectionReason",
        )?;
        assert_reflected_camera_payload::<CameraRequestPreparationError>(
            &registry,
            "hana_lagrange::animation::events::CameraRequestPreparationError",
        )?;
        assert_reflected_camera_payload::<CameraEventTiming>(
            &registry,
            "hana_lagrange::animation::events::CameraEventTiming",
        )?;
        assert_reflected_camera_payload::<CameraMoveError>(
            &registry,
            "hana_lagrange::animation::queue::CameraMoveError",
        )?;
        assert_reflected_camera_payload::<CameraEvaluationError>(
            &registry,
            "hana_lagrange::animation::sequence::CameraEvaluationError",
        )?;
        assert_reflected_camera_payload::<ZoomContext>(
            &registry,
            "hana_lagrange::fit::triggers::zoom::ZoomContext",
        )?;
        assert_reflected_camera_payload::<ZoomReason>(
            &registry,
            "hana_lagrange::fit::triggers::zoom::ZoomReason",
        )?;
        assert_reflected_camera_payload::<AnimationConflictPolicy>(
            &registry,
            "hana_lagrange::animation::lifecycle::AnimationConflictPolicy",
        )?;

        let request = registry
            .get(TypeId::of::<PlayAnimation>())
            .ok_or("PlayAnimation was not automatically registered")?;
        let sequence = registry
            .get(TypeId::of::<CameraSequence>())
            .ok_or("CameraSequence was not automatically registered")?;
        assert!(matches!(request.type_info(), TypeInfo::Struct(_)));
        assert!(matches!(sequence.type_info(), TypeInfo::Opaque(_)));
        assert_eq!(
            sequence.type_info().type_path(),
            "hana_lagrange::animation::sequence::CameraSequence"
        );
        drop(registry);
        Ok(())
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one regression covers every camera event and its nested opaque payloads"
    )]
    fn camera_events_serialize_through_the_type_registry() -> TestResult {
        let app = camera_sequence_test_app();
        let registry = app.world().resource::<AppTypeRegistry>().read();
        let camera = Entity::from_bits(1);
        let target = Entity::from_bits(2);
        let sequence = three_move_sequence();
        let camera_move = sequence.moves()[0].clone();
        let stage_id = sequence
            .stage_ids_with_spans()
            .next()
            .ok_or("missing stage")?
            .0;
        let total = sequence.total();
        let timing = CameraEventTiming::new(SequencePosition::END, total);
        let owner = SequenceOwner::NativePlayback;
        let direction = SequenceDirection::Forward;
        let source = AnimationSource::PlayAnimation;
        let events: [&dyn Reflect; 10] = [
            &AnimationEnd {
                camera,
                source,
                target: Some(target),
                owner,
                direction,
                timing,
                reason: AnimationReason::Completed,
            },
            &AnimationEnd {
                camera,
                source,
                target: Some(target),
                owner,
                direction,
                timing,
                reason: AnimationReason::Cancelled {
                    interrupted_stage_id: stage_id,
                    interrupted_move:     camera_move.clone(),
                },
            },
            &PlayAnimation::new(camera, [camera_move.clone()]),
            &AnimationBegin {
                camera,
                source,
                target: Some(target),
                owner,
                direction,
                timing,
            },
            &AnimationRejected {
                camera,
                source,
                target: Some(target),
                reason: AnimationRejectionReason::InvalidMove(CameraMoveError::NegativeRadius {
                    radius: crate::Radius(-1.0),
                }),
            },
            &AnimationRejected {
                camera,
                source,
                target: Some(target),
                reason: AnimationRejectionReason::PreparationFailed(
                    CameraEvaluationError::ExternalCurveRejected(
                        SequenceEasingError::MappingNotBoundedMonotonic,
                    ),
                ),
            },
            &CameraMoveBegin {
                camera,
                stage_id,
                owner,
                direction,
                camera_move: camera_move.clone(),
                boundary_elapsed: total,
                timing,
            },
            &CameraMoveEnd {
                camera,
                stage_id,
                owner,
                direction,
                camera_move,
                boundary_elapsed: total,
                timing,
            },
            &ZoomBegin {
                camera,
                target,
                margin: 0.1,
                duration: Duration::from_secs(1),
                easing: EaseFunction::Linear,
            },
            &ZoomEnd {
                camera,
                target,
                margin: 0.1,
                duration: Duration::from_secs(1),
                easing: EaseFunction::Linear,
                reason: ZoomReason::Completed,
            },
        ];

        for event in events {
            let path = event.reflect_type_path();
            let result = serde_json::to_value(ReflectSerializer::new(event, &registry));
            assert!(result.is_ok(), "{path} failed to serialize: {result:?}");
            let value = result.map_err(|_| "camera event serialization failed")?;
            assert!(
                value.get(path).is_some(),
                "{path} was not keyed by its type path: {value}"
            );
            if path == "hana_lagrange::animation::events::AnimationEnd" {
                assert_eq!(value[path]["timing"]["normalized_position"], 1.0);
                assert_eq!(
                    value[path]["timing"]["total"]["whole_seconds"],
                    total.whole_seconds().to_string(),
                );
                assert_eq!(
                    value[path]["timing"]["total"]["subsec_nanoseconds"],
                    total.subsec_nanoseconds(),
                );
            }
        }
        drop(registry);
        Ok(())
    }

    #[test]
    fn opaque_reflection_cannot_mutate_sequence_authoring_or_derived_stages() {
        let sequence = three_move_sequence();
        let reflected = &sequence as &dyn Reflect;

        assert!(matches!(reflected.reflect_ref(), ReflectRef::Opaque(_)));
        assert_eq!(sequence.moves().len(), 3);
        assert_eq!(sequence.stage_ids_with_spans().count(), 3);
    }

    #[test]
    fn a_sequence_built_by_chaining_retains_every_move_in_play_order() {
        let sequence = three_move_sequence();

        assert_eq!(sequence.moves().len(), 3);
        assert_eq!(
            sequence.moves()[0].duration(),
            Duration::from_millis(FIRST_MOVE_MILLIS)
        );
        assert_eq!(
            sequence.moves()[1].duration(),
            Duration::from_millis(SECOND_MOVE_MILLIS)
        );
        assert_eq!(
            sequence.moves()[2].duration(),
            Duration::from_millis(THIRD_MOVE_MILLIS)
        );
    }

    #[test]
    fn collecting_no_move_reports_the_empty_iterator() {
        assert_eq!(
            CameraSequence::try_from_moves([]),
            Err(CameraSequenceError::NoMoves)
        );
    }

    #[test]
    fn collecting_at_least_one_move_builds_the_same_sequence_as_chaining() {
        let collected = CameraSequence::try_from_moves([
            move_lasting(Duration::from_millis(FIRST_MOVE_MILLIS)),
            move_lasting(Duration::from_millis(SECOND_MOVE_MILLIS)),
            move_lasting(Duration::from_millis(THIRD_MOVE_MILLIS)),
        ]);

        assert_eq!(collected, Ok(three_move_sequence()));
    }

    #[test]
    fn stage_identities_and_spans_pair_one_to_one_with_the_authored_moves() -> TestResult {
        let sequence = three_move_sequence();

        let paired: Vec<_> = sequence.stage_ids_with_spans().collect();
        assert_eq!(paired.len(), sequence.moves().len());
        let mut authored_so_far: Vec<Duration> = Vec::new();
        for ((stage_id, span), camera_move) in paired.iter().zip(sequence.moves()) {
            let Ok(looked_up) = sequence.sequence_stages().span(*stage_id) else {
                return Err("every paired identity resolves against its own description");
            };
            assert_eq!(looked_up, *span);
            assert_eq!(span.start(), SequenceTime::sum(&authored_so_far));
            authored_so_far.push(camera_move.duration());
            assert_eq!(span.end(), SequenceTime::sum(&authored_so_far));
        }

        Ok(())
    }

    #[test]
    fn a_coincident_zero_duration_boundary_survives_construction() -> TestResult {
        let sequence = three_move_sequence();

        let spans: Vec<_> = sequence
            .stage_ids_with_spans()
            .map(|(_, span)| span)
            .collect();
        let Some(zero_span) = spans.get(1) else {
            return Err("the zero-duration move keeps its own stage");
        };
        assert_eq!(zero_span.start(), zero_span.end());
        assert_eq!(
            zero_span.start(),
            SequenceTime::from(Duration::from_millis(FIRST_MOVE_MILLIS))
        );

        Ok(())
    }

    #[test]
    fn the_sequence_total_is_the_exact_sum_of_every_authored_duration() {
        let sequence = three_move_sequence();

        let durations: Vec<Duration> = sequence.moves().iter().map(CameraMove::duration).collect();
        assert_eq!(sequence.total(), SequenceTime::sum(&durations));
    }

    #[test]
    fn equality_reads_the_authored_moves_and_not_the_stage_revision() {
        assert_eq!(three_move_sequence(), three_move_sequence());
        assert_ne!(
            three_move_sequence(),
            three_move_sequence().then(move_lasting(Duration::from_millis(THIRD_MOVE_MILLIS)))
        );
    }

    #[test]
    fn replacing_the_authored_definition_draws_a_fresh_stage_revision() -> TestResult {
        let sequence = three_move_sequence();
        let Some((first_stage_id, _)) = sequence.stage_ids_with_spans().next() else {
            return Err("a three-move sequence names a first stage");
        };

        let replaced = sequence.then(move_lasting(Duration::from_millis(FIRST_MOVE_MILLIS)));

        assert!(replaced.sequence_stages().span(first_stage_id).is_err());

        Ok(())
    }
}
