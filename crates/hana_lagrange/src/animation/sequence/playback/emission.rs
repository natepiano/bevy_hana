use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::warn;
use hana_kana::SequenceDirection;
use hana_kana::SequenceOwner;
use hana_kana::SequenceStageId;
use hana_kana::SequenceUpdate;

use super::CameraPlaybackClosureReason;
use super::CameraPlaybackLifecycleState;
use super::CameraSequencePlayback;
use super::DriverEpisode;
use super::camera_journey_matches;
use super::camera_lifecycle_owner_and_direction;
use super::ledger;
use super::ledger::CameraMoveBoundary;
use crate::animation::events::AnimationBegin;
use crate::animation::events::AnimationEnd;
use crate::animation::events::AnimationReason;
use crate::animation::events::CameraEventTiming;
use crate::animation::events::CameraMoveBegin;
use crate::animation::events::CameraMoveEnd;
use crate::animation::queue::CameraMove;
use crate::animation::sequence::CameraSequence;
use crate::animation::sequence::RetainedCameraJourney;
use crate::fit::ZoomBegin;
use crate::fit::ZoomEnd;
use crate::fit::ZoomReason;

pub(super) fn emit_camera_lifecycle_begin(
    commands: &mut Commands,
    camera: Entity,
    sequence: &CameraSequence,
    retained: &CameraSequencePlayback,
    journey: &RetainedCameraJourney,
    owner: SequenceOwner,
    direction: SequenceDirection,
) {
    if !camera_journey_matches(sequence, retained, journey) {
        return;
    }
    if let Some(zoom) = journey.zoom.as_ref() {
        commands.trigger(ZoomBegin {
            camera,
            target: zoom.target,
            margin: zoom.margin,
            duration: zoom.duration,
            easing: zoom.easing,
        });
    }
    commands.trigger(AnimationBegin {
        camera,
        source: journey.origin.event_source(),
        target: journey.target,
        owner,
        direction,
        timing: CameraEventTiming::new(retained.playback.position(), sequence.total()),
    });
}

pub(in crate::animation::sequence) fn emit_cancelled_camera_lifecycle(
    commands: &mut Commands,
    camera: Entity,
    sequence: &CameraSequence,
    retained: &CameraSequencePlayback,
    journey: &RetainedCameraJourney,
) {
    if !matches!(
        retained.lifecycle,
        CameraPlaybackLifecycleState::Effective { .. }
            | CameraPlaybackLifecycleState::Closing { .. }
    ) || (matches!(
        retained.lifecycle,
        CameraPlaybackLifecycleState::Effective {
            owner: SequenceOwner::Driver(_),
        } | CameraPlaybackLifecycleState::Closing {
            owner: SequenceOwner::Driver(_),
            ..
        }
    ) && matches!(retained.driver_episode, DriverEpisode::Closed))
        || !camera_journey_matches(sequence, retained, journey)
    {
        return;
    }
    emit_camera_lifecycle_end(
        commands,
        camera,
        sequence,
        retained,
        journey,
        CameraPlaybackClosureReason::Cancelled,
    );
}

pub(super) fn emit_camera_lifecycle_end(
    commands: &mut Commands,
    camera: Entity,
    sequence: &CameraSequence,
    retained: &CameraSequencePlayback,
    journey: &RetainedCameraJourney,
    reason: CameraPlaybackClosureReason,
) {
    if !camera_journey_matches(sequence, retained, journey) {
        return;
    }
    let zoom_reason = match reason {
        CameraPlaybackClosureReason::Completed => ZoomReason::Completed,
        CameraPlaybackClosureReason::Cancelled => ZoomReason::Cancelled,
    };
    let animation_reason = match reason {
        CameraPlaybackClosureReason::Completed => AnimationReason::Completed,
        CameraPlaybackClosureReason::Cancelled => {
            let Some((interrupted_stage_id, interrupted_move)) =
                interrupted_camera_move(sequence, retained)
            else {
                return;
            };
            AnimationReason::Cancelled {
                interrupted_stage_id,
                interrupted_move,
            }
        },
    };
    let (owner, direction) = camera_lifecycle_owner_and_direction(retained);
    commands.trigger(AnimationEnd {
        camera,
        source: journey.origin.event_source(),
        target: journey.target,
        owner,
        direction,
        timing: CameraEventTiming::new(retained.playback.position(), sequence.total()),
        reason: animation_reason,
    });
    if let Some(zoom) = journey.zoom.as_ref() {
        commands.trigger(ZoomEnd {
            camera,
            target: zoom.target,
            margin: zoom.margin,
            duration: zoom.duration,
            easing: zoom.easing,
            reason: zoom_reason,
        });
    }
}

fn interrupted_camera_move(
    sequence: &CameraSequence,
    retained: &CameraSequencePlayback,
) -> Option<(SequenceStageId, CameraMove)> {
    let position = f64::from(retained.playback.position().normalized());
    let ordinal = retained
        .endpoints
        .iter()
        .position(|endpoint| position <= endpoint.interval.end)
        .unwrap_or_else(|| sequence.moves().len().saturating_sub(1));
    let endpoint = retained.endpoints.get(ordinal)?;
    sequence
        .moves()
        .get(ordinal)
        .cloned()
        .map(|camera_move| (endpoint.stage_id, camera_move))
}

pub(super) fn emit_camera_boundaries(
    commands: &mut Commands,
    camera: Entity,
    sequence: &CameraSequence,
    retained: &mut CameraSequencePlayback,
    journey: Option<&RetainedCameraJourney>,
    owner: SequenceOwner,
    update: SequenceUpdate,
) {
    let SequenceUpdate::Traversed {
        traversal,
        direction,
        ..
    } = update
    else {
        return;
    };
    let mut ordinals = traversal.boundaries().peekable();
    while let Some(ordinal) = ordinals.next() {
        let Some(record) = retained.boundary_ledger.records.get(ordinal) else {
            warn!(camera = ?camera, ordinal, "camera traversal referenced no retained boundary record");
            continue;
        };
        if matches!(owner, SequenceOwner::Driver(_))
            && matches!(retained.lifecycle, CameraPlaybackLifecycleState::Dormant)
            && matches!(
                camera_boundary_lifecycle(record.boundary, direction, sequence.moves().len()),
                CameraBoundaryLifecycle::CycleStart
            )
            && let Some(journey) = journey
        {
            emit_camera_lifecycle_begin(
                commands, camera, sequence, retained, journey, owner, direction,
            );
            retained.lifecycle = CameraPlaybackLifecycleState::Effective { owner };
            retained.driver_episode = DriverEpisode::Open;
            retained.interruption.rearm();
        }
        let stage_ordinal = match record.boundary {
            CameraMoveBoundary::Begin { stage_ordinal, .. }
            | CameraMoveBoundary::End { stage_ordinal, .. } => stage_ordinal,
        };
        let Some(camera_move) = sequence.moves().get(stage_ordinal).cloned() else {
            warn!(camera = ?camera, stage_ordinal, "camera traversal referenced no authored move");
            continue;
        };
        let begins = matches!(
            (record.boundary, direction),
            (
                CameraMoveBoundary::Begin { .. },
                hana_kana::SequenceDirection::Forward
            ) | (
                CameraMoveBoundary::End { .. },
                hana_kana::SequenceDirection::Backward
            )
        );
        if begins {
            commands.trigger(CameraMoveBegin {
                camera,
                stage_id: ledger::stage_id_from_boundary(record.boundary),
                owner,
                direction,
                camera_move,
                boundary_elapsed: record.boundary_elapsed,
                timing: CameraEventTiming::new(record.position, sequence.total()),
            });
        } else {
            commands.trigger(CameraMoveEnd {
                camera,
                stage_id: ledger::stage_id_from_boundary(record.boundary),
                owner,
                direction,
                camera_move,
                boundary_elapsed: record.boundary_elapsed,
                timing: CameraEventTiming::new(record.position, sequence.total()),
            });
        }

        let boundary_lifecycle =
            camera_boundary_lifecycle(record.boundary, direction, sequence.moves().len());
        if matches!(owner, SequenceOwner::Driver(_))
            && matches!(
                retained.lifecycle,
                CameraPlaybackLifecycleState::Effective { .. }
            )
            && matches!(retained.driver_episode, DriverEpisode::Open)
            && matches!(boundary_lifecycle, CameraBoundaryLifecycle::Completion)
        {
            if ordinals.peek().is_some() {
                if let Some(journey) = journey {
                    emit_camera_lifecycle_end(
                        commands,
                        camera,
                        sequence,
                        retained,
                        journey,
                        CameraPlaybackClosureReason::Completed,
                    );
                }
                retained.lifecycle = CameraPlaybackLifecycleState::Dormant;
                retained.driver_episode = DriverEpisode::Closed;
                retained.interruption.rearm();
            } else {
                retained.lifecycle = CameraPlaybackLifecycleState::Closing {
                    owner,
                    reason: CameraPlaybackClosureReason::Completed,
                };
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CameraBoundaryLifecycle {
    CycleStart,
    Completion,
    Interior,
}

const fn camera_boundary_lifecycle(
    boundary: CameraMoveBoundary,
    direction: SequenceDirection,
    move_count: usize,
) -> CameraBoundaryLifecycle {
    if matches!(
        (boundary, direction),
        (CameraMoveBoundary::Begin { .. }, SequenceDirection::Forward)
            | (CameraMoveBoundary::End { .. }, SequenceDirection::Backward)
    ) {
        return CameraBoundaryLifecycle::CycleStart;
    }
    match (boundary, direction) {
        (CameraMoveBoundary::End { stage_ordinal, .. }, SequenceDirection::Forward) => {
            if stage_ordinal.saturating_add(1) == move_count {
                CameraBoundaryLifecycle::Completion
            } else {
                CameraBoundaryLifecycle::Interior
            }
        },
        (
            CameraMoveBoundary::Begin {
                stage_ordinal: 0, ..
            },
            SequenceDirection::Backward,
        ) => CameraBoundaryLifecycle::Completion,
        _ => CameraBoundaryLifecycle::Interior,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use bevy::math::curve::easing::EaseFunction;
    use bevy::prelude::Camera;
    use bevy::prelude::OrthographicProjection;
    use bevy::prelude::Projection;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec3;
    use bevy::reflect::ReflectRef;
    use hana_kana::RangeCrossing;
    use hana_kana::RangeCrossings;
    use hana_kana::RangeEdge;
    use hana_kana::SequenceMovement;
    use hana_kana::SequencePosition;
    use hana_kana::SequenceStages;

    use super::*;
    use crate::AnimateToFit;
    use crate::AnimationSource;
    use crate::CameraBasis;
    use crate::CurrentFitTarget;
    use crate::FreeCam;
    use crate::LookAt;
    use crate::LookAtAndZoomToFit;
    use crate::PlayAnimation;
    use crate::ZoomContext;
    use crate::ZoomToFit;
    use crate::animation::sequence::RetainedCameraJourneyOrigin;
    use crate::animation::sequence::playback::pose;
    use crate::animation::sequence::playback::*;
    use crate::animation::sequence::support::*;

    #[test]
    fn selected_driver_large_multi_wrap_traces_every_lifecycle_and_boundary_in_raw_order()
    -> TestResult {
        const REPETITIONS: i64 = 32;
        let mut app = camera_event_trace_app();
        let sequence = three_move_sequence();
        let stages: Vec<_> = sequence.stage_ids_with_spans().collect();
        let [first_stage, second_stage, third_stage] = stages.as_slice() else {
            return Err("the three-move sequence must retain three stable stage identities");
        };
        let total = sequence.total();
        let camera = app
            .world_mut()
            .spawn((orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0), sequence))
            .id();
        let crossings = RangeCrossings::try_new((0..REPETITIONS).map(|ordinal| {
            RangeCrossing::new(
                if ordinal % 2 == 0 {
                    RangeEdge::End
                } else {
                    RangeEdge::Start
                },
                SequenceDirection::Forward,
            )
        }))
        .map_err(|_| "the alternating maximum-size crossing record is valid")?;
        let movement = SequenceMovement::try_new(
            SequencePosition::END,
            SequenceDirection::Forward,
            REPETITIONS,
            crossings,
        )
        .map_err(|_| "the crossing record describes every forward repetition")?;
        let driver = app
            .world_mut()
            .spawn((hana_kana::SequenceDriver::new(camera), movement))
            .id();

        app.update();

        let owner = SequenceOwner::Driver(driver);
        let start_timing = CameraEventTiming::new(SequencePosition::START, total);
        let middle_position = SequencePosition::try_new(0.25)
            .map_err(|_| "one quarter is a valid normalized sequence position")?;
        let middle_timing = CameraEventTiming::new(middle_position, total);
        let end_timing = CameraEventTiming::new(SequencePosition::END, total);
        let expected_boundaries = forward_three_move_boundaries(
            camera,
            owner,
            [*first_stage, *second_stage, *third_stage],
            [start_timing, middle_timing, end_timing],
        );
        let mut expected = Vec::with_capacity(
            usize::try_from(REPETITIONS + 1).expect("the test repetition count fits usize")
                * (expected_boundaries.len() + 2),
        );
        for _ in 0..=REPETITIONS {
            expected.push(CameraEventTraceItem::AnimationBegin {
                camera,
                source: AnimationSource::CameraSequence,
                target: None,
                owner,
                direction: SequenceDirection::Forward,
                timing: end_timing,
            });
            expected.extend(expected_boundaries.iter().cloned());
            expected.push(CameraEventTraceItem::AnimationEnd {
                camera,
                source: AnimationSource::CameraSequence,
                target: None,
                owner,
                direction: SequenceDirection::Forward,
                timing: end_timing,
                outcome: CameraEventOutcome::Completed,
            });
        }

        let trace = &app.world().resource::<CameraEventTrace>().0;
        assert_eq!(
            trace.len(),
            usize::try_from(REPETITIONS + 1).expect("the test repetition count fits usize")
                * (expected_boundaries.len() + 2)
        );
        assert_eq!(trace, &expected);
        Ok(())
    }

    #[test]
    fn selected_driver_backward_large_multi_wrap_emits_every_boundary_in_raw_order() -> TestResult {
        const REPETITIONS: i64 = -32;
        let mut app = camera_sequence_test_app();
        app.init_resource::<CameraBoundaryEventOrder>();
        let camera = app
            .world_mut()
            .spawn((
                orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0),
                three_move_sequence(),
            ))
            .id();
        record_camera_boundary_order(app.world_mut(), camera);
        let driver = app
            .world_mut()
            .spawn((
                hana_kana::SequenceDriver::new(camera),
                SequenceMovement::try_new(
                    SequencePosition::END,
                    SequenceDirection::Forward,
                    0,
                    RangeCrossings::NONE,
                )
                .map_err(|_| "the forward endpoint fixture movement is valid")?,
            ))
            .id();
        app.update();
        app.world_mut()
            .resource_mut::<CameraBoundaryEventOrder>()
            .0
            .clear();

        let crossings = RangeCrossings::try_new((0..REPETITIONS.unsigned_abs()).map(|ordinal| {
            RangeCrossing::new(
                if ordinal % 2 == 0 {
                    RangeEdge::Start
                } else {
                    RangeEdge::End
                },
                SequenceDirection::Backward,
            )
        }))
        .map_err(|_| "the alternating backward crossing record is valid")?;
        let movement = SequenceMovement::try_new(
            SequencePosition::START,
            SequenceDirection::Backward,
            REPETITIONS,
            crossings,
        )
        .map_err(|_| "the crossing record describes every backward repetition")?;
        app.world_mut().entity_mut(driver).insert(movement);

        app.update();

        let order = &app.world().resource::<CameraBoundaryEventOrder>().0;
        let expected_cycle = [
            ("begin", Duration::from_millis(THIRD_MOVE_MILLIS)),
            ("end", Duration::from_millis(THIRD_MOVE_MILLIS)),
            ("begin", Duration::from_millis(SECOND_MOVE_MILLIS)),
            ("end", Duration::from_millis(SECOND_MOVE_MILLIS)),
            ("begin", Duration::from_millis(FIRST_MOVE_MILLIS)),
            ("end", Duration::from_millis(FIRST_MOVE_MILLIS)),
        ];
        assert_eq!(
            order.len(),
            usize::try_from(REPETITIONS.unsigned_abs() + 1)
                .expect("the test repetition count fits usize")
                * expected_cycle.len()
        );
        assert!(
            order
                .chunks_exact(expected_cycle.len())
                .all(|cycle| cycle == expected_cycle)
        );
        Ok(())
    }

    #[test]
    fn zero_duration_zoom_to_fit_traces_zoom_then_retained_completion() -> TestResult {
        let mut app = camera_event_trace_app();
        let position = Vec3::new(0.0, 0.0, 8.0);
        let target = spawn_zero_duration_facade_target(&mut app, Vec3::ZERO);
        let camera = spawn_zero_duration_free_camera(&mut app, position);

        app.world_mut().trigger(ZoomToFit::new(camera, target));
        app.update();

        let free = app
            .world()
            .get::<FreeCam>(camera)
            .ok_or("ZoomToFit retains its free-flight camera")?;
        assert_approximately_equal(free.look.current().yaw, 0.25);
        assert_approximately_equal(free.look.current().pitch, -0.1);
        assert_approximately_equal(free.roll.current().0, 0.35);
        assert_ne!(free.translate.current().0, position);
        assert_eq!(
            app.world()
                .get::<CurrentFitTarget>(camera)
                .map(|current| current.0),
            Some(target)
        );
        assert_zero_duration_native_facade_trace(
            &app,
            camera,
            target,
            AnimationSource::ZoomToFit,
            ZeroDurationFacadeTraceShape::ZoomWithOneAuthoredMove,
        )
    }

    #[test]
    fn zero_duration_animate_to_fit_traces_retained_completion() -> TestResult {
        let mut app = camera_event_trace_app();
        let position = Vec3::new(0.0, 0.0, 8.0);
        let target = spawn_zero_duration_facade_target(&mut app, Vec3::ZERO);
        let camera = spawn_zero_duration_free_camera(&mut app, position);

        app.world_mut()
            .trigger(AnimateToFit::new(camera, target).yaw(0.4).pitch(-0.2));
        app.update();

        let free = app
            .world()
            .get::<FreeCam>(camera)
            .ok_or("AnimateToFit retains its free-flight camera")?;
        assert_approximately_equal(free.look.current().yaw, 0.4);
        assert_approximately_equal(free.look.current().pitch, -0.2);
        assert_approximately_equal(free.roll.current().0, 0.0);
        assert_ne!(free.translate.current().0, position);
        assert_eq!(
            app.world()
                .get::<CurrentFitTarget>(camera)
                .map(|current| current.0),
            Some(target)
        );
        assert_zero_duration_native_facade_trace(
            &app,
            camera,
            target,
            AnimationSource::AnimateToFit,
            ZeroDurationFacadeTraceShape::OneAuthoredMove,
        )
    }

    #[test]
    fn zero_duration_look_at_traces_retained_completion() -> TestResult {
        let mut app = camera_event_trace_app();
        let position = Vec3::new(0.0, 0.0, 8.0);
        let target_position = Vec3::new(2.0, 1.0, 0.0);
        let target = spawn_zero_duration_facade_target(&mut app, target_position);
        let camera = spawn_zero_duration_free_camera(&mut app, position);

        app.world_mut().trigger(LookAt::new(camera, target));
        app.update();

        let expected = pose::free_camera_look_at(position, target_position, CameraBasis::Y_UP);
        let free = app
            .world()
            .get::<FreeCam>(camera)
            .ok_or("LookAt retains its free-flight camera")?;
        assert_eq!(free.translate.current().0, position);
        assert_approximately_equal(free.look.current().yaw, expected.yaw);
        assert_approximately_equal(free.look.current().pitch, expected.pitch);
        assert_approximately_equal(free.roll.current().0, 0.35);
        assert!(app.world().get::<CurrentFitTarget>(camera).is_none());
        assert_zero_duration_native_facade_trace(
            &app,
            camera,
            target,
            AnimationSource::LookAt,
            ZeroDurationFacadeTraceShape::OneAuthoredMove,
        )
    }

    #[test]
    fn zero_duration_look_at_and_zoom_to_fit_traces_retained_completion() -> TestResult {
        let mut app = camera_event_trace_app();
        let position = Vec3::new(0.0, 0.0, 8.0);
        let target_position = Vec3::new(2.0, 1.0, 0.0);
        let target = spawn_zero_duration_facade_target(&mut app, target_position);
        let camera = spawn_zero_duration_free_camera(&mut app, position);

        app.world_mut()
            .trigger(LookAtAndZoomToFit::new(camera, target).margin(0.2));
        app.update();

        let expected = pose::free_camera_look_at(position, target_position, CameraBasis::Y_UP);
        let free = app
            .world()
            .get::<FreeCam>(camera)
            .ok_or("LookAtAndZoomToFit retains its free-flight camera")?;
        assert_approximately_equal(free.look.current().yaw, expected.yaw);
        assert_approximately_equal(free.look.current().pitch, expected.pitch);
        assert_approximately_equal(free.roll.current().0, 0.35);
        assert_ne!(free.translate.current().0, position);
        assert_eq!(
            app.world()
                .get::<CurrentFitTarget>(camera)
                .map(|current| current.0),
            Some(target)
        );
        assert_zero_duration_native_facade_trace(
            &app,
            camera,
            target,
            AnimationSource::LookAtAndZoomToFit,
            ZeroDurationFacadeTraceShape::TwoAuthoredMoves,
        )
    }

    #[test]
    fn facade_metadata_survives_basis_loss_and_selected_recovery() -> TestResult {
        let mut app = camera_sequence_test_app();
        app.init_resource::<AnimationBeginMetadata>()
            .init_resource::<ZoomBeginMetadata>()
            .add_observer(record_animation_begin_metadata)
            .add_observer(record_zoom_begin_metadata);
        let target = app.world_mut().spawn_empty().id();
        let camera = app
            .world_mut()
            .spawn((
                free_camera(Vec3::new(0.0, 0.0, 10.0), 0.0, 0.0, 0.0),
                CameraBasis::Y_UP,
                Camera::default(),
                Projection::Orthographic(OrthographicProjection {
                    scale: 3.0,
                    ..OrthographicProjection::default_3d()
                }),
                Transform::from_xyz(0.0, 0.0, 10.0),
            ))
            .id();
        let zoom = ZoomContext {
            target,
            margin: 0.27,
            duration: Duration::from_secs(3),
            easing: EaseFunction::SineInOut,
        };
        app.world_mut().trigger(
            PlayAnimation::new(camera, [move_lasting(Duration::from_secs(10))])
                .zoom_context(zoom)
                .target(target),
        );
        app.update();

        let revision = app
            .world()
            .get::<CameraSequence>(camera)
            .ok_or("the accepted facade request retains authoring")?
            .sequence_stages()
            .revision();
        assert_facade_journey_metadata(app.world(), camera, revision, target)?;

        app.world_mut().entity_mut(camera).remove::<CameraBasis>();
        app.update();

        assert!(app.world().get::<CameraSequencePlayback>(camera).is_none());
        assert!(app.world().get::<SequenceStages>(camera).is_none());
        assert_facade_journey_metadata(app.world(), camera, revision, target)?;

        app.world_mut()
            .spawn(hana_kana::SequenceDriver::new(camera));
        app.update();
        app.world_mut().entity_mut(camera).insert(CameraBasis::Y_UP);
        app.update();

        assert_facade_journey_metadata(app.world(), camera, revision, target)?;
        assert_eq!(
            app.world().resource::<AnimationBeginMetadata>().0,
            [(AnimationSource::ZoomToFit, Some(target))]
        );
        assert_eq!(
            app.world().resource::<ZoomBeginMetadata>().0,
            [(
                target,
                0.27,
                Duration::from_secs(3),
                EaseFunction::SineInOut
            )]
        );
        Ok(())
    }

    #[test]
    fn direct_and_facade_journeys_have_distinct_private_origins() -> TestResult {
        let mut app = camera_sequence_test_app();
        let direct = app
            .world_mut()
            .spawn((
                orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0),
                CameraSequence::new(move_lasting(Duration::from_secs(1))),
            ))
            .id();
        let facade = app
            .world_mut()
            .spawn(orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0))
            .id();
        app.world_mut().trigger(PlayAnimation::new(
            facade,
            [move_lasting(Duration::from_secs(1))],
        ));
        app.update();

        assert!(matches!(
            app.world()
                .get::<RetainedCameraJourney>(direct)
                .ok_or("direct authoring retains private journey metadata")?
                .origin,
            RetainedCameraJourneyOrigin::DirectCameraSequence
        ));
        assert!(matches!(
            app.world()
                .get::<RetainedCameraJourney>(facade)
                .ok_or("the facade retains private journey metadata")?
                .origin,
            RetainedCameraJourneyOrigin::Facade(AnimationSource::PlayAnimation)
        ));
        Ok(())
    }

    #[test]
    fn camera_event_timing_remains_opaque_with_read_only_accessors() {
        let total = three_move_sequence().total();
        let timing = CameraEventTiming::new(SequencePosition::START, total);

        assert_eq!(timing.position(), SequencePosition::START);
        assert_eq!(timing.total(), total);
        assert!(matches!(
            (&timing as &dyn Reflect).reflect_ref(),
            ReflectRef::Opaque(_)
        ));
    }
}
