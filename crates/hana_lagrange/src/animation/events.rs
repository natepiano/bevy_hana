#![allow(
    clippy::used_underscore_binding,
    reason = "false positive on enum variant fields"
)]

use std::collections::VecDeque;

use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::ReflectFromReflect;
use hana_kana::SequenceDirection;
use hana_kana::SequenceOwner;
use hana_kana::SequencePosition;
use hana_kana::SequenceStageId;
use hana_kana::SequenceTime;

use super::lifecycle::AnimationConflictPolicy;
use super::queue::CameraMove;
use super::queue::CameraMoveError;
use super::sequence::CameraEvaluationError;
use crate::fit::ZoomContext;

/// Identifies which event triggered an animation lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum AnimationSource {
    /// Animation was triggered by `PlayAnimation`.
    PlayAnimation,
    /// Animation was triggered by `ZoomToFit`.
    ZoomToFit,
    /// Animation was triggered by `AnimateToFit`.
    AnimateToFit,
    /// Animation was triggered by `LookAt`.
    LookAt,
    /// Animation was triggered by `LookAtAndZoomToFit`.
    LookAtAndZoomToFit,
    /// Animation was triggered by direct retained `CameraSequence` authoring.
    CameraSequence,
}

/// `CameraEventTiming` captures raw retained playback timing for one camera event.
///
/// The normalized position identifies traversal ordering while `total` keeps
/// the exact authored duration available.  It intentionally carries no
/// reconstructed elapsed time: an interior normalized value cannot identify
/// one exact authored nanosecond.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(opaque)]
pub struct CameraEventTiming {
    position: SequencePosition,
    total:    SequenceTime,
}

impl CameraEventTiming {
    /// Creates a `CameraEventTiming` snapshot for an event payload.
    #[must_use]
    pub const fn new(position: SequencePosition, total: SequenceTime) -> Self {
        Self { position, total }
    }

    /// Returns the raw retained normalized position.
    #[must_use]
    pub const fn position(self) -> SequencePosition { self.position }

    /// Returns the exact total authored duration.
    #[must_use]
    pub const fn total(self) -> SequenceTime { self.total }
}

/// Requests retained playback of an ordered sequence of [`CameraMove`] values.
///
/// Accepted requests compile into the same retained [`CameraSequence`](crate::CameraSequence)
/// backend used by direct authoring and are controlled through the shared
/// camera command and driver-ownership model.
#[derive(EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct PlayAnimation {
    /// The camera entity to animate.
    #[event_target]
    pub camera:       Entity,
    /// The ordered camera movements to compile into retained authoring.
    pub camera_moves: VecDeque<CameraMove>,
    /// The source of this animation.
    pub source:       AnimationSource,
    /// The entity this animation frames, when it frames one (`AnimateToFit`,
    /// `LookAt`, `LookAtAndZoomToFit`, `ZoomToFit`); `None` for a raw
    /// `PlayAnimation`. Surfaced on the lifecycle events so observers can tell
    /// which target an animation was for.
    pub target:       Option<Entity>,
    /// Optional zoom context when this animation originates from `ZoomToFit`.
    pub zoom_context: Option<ZoomContext>,
}

impl PlayAnimation {
    /// Creates a new `PlayAnimation` event.
    #[must_use]
    pub fn new(camera: Entity, camera_moves: impl IntoIterator<Item = CameraMove>) -> Self {
        Self {
            camera,
            camera_moves: camera_moves.into_iter().collect(),
            source: AnimationSource::PlayAnimation,
            target: None,
            zoom_context: None,
        }
    }

    /// Sets the animation source.
    #[must_use]
    pub const fn source(mut self, source: AnimationSource) -> Self {
        self.source = source;
        self
    }

    /// Sets the entity this animation frames. The target is surfaced on the
    /// `AnimationBegin` / `AnimationEnd` / `AnimationRejected` events so
    /// observers can distinguish which target an animation was for.
    #[must_use]
    pub const fn target(mut self, target: Entity) -> Self {
        self.target = Some(target);
        self
    }

    /// Sets the zoom context and marks the source as `ZoomToFit`.
    #[must_use]
    pub const fn zoom_context(mut self, zoom_context: ZoomContext) -> Self {
        self.zoom_context = Some(zoom_context);
        self.source = AnimationSource::ZoomToFit;
        self
    }
}

/// Emitted when a retained camera journey becomes effective.
#[derive(EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct AnimationBegin {
    /// The camera being animated.
    #[event_target]
    pub camera:    Entity,
    /// Whether this animation originated from `PlayAnimation`, `ZoomToFit`, `AnimateToFit`,
    /// `LookAt`, or `LookAtAndZoomToFit`.
    pub source:    AnimationSource,
    /// The entity this animation frames, or `None` for a raw `PlayAnimation`.
    pub target:    Option<Entity>,
    /// Playback owner that made the journey effective.
    pub owner:     SequenceOwner,
    /// Direction that opened this lifecycle episode.
    pub direction: SequenceDirection,
    /// Raw retained playback timing at admission or first driver traversal.
    pub timing:    CameraEventTiming,
}

/// Emitted when an animation stops running, either by completing naturally or
/// by being cancelled. Inspect [`AnimationEnd::reason`] to distinguish.
#[derive(EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct AnimationEnd {
    /// The camera that stopped animating.
    #[event_target]
    pub camera:    Entity,
    /// Whether this animation originated from `PlayAnimation`, `ZoomToFit`, `AnimateToFit`,
    /// `LookAt`, or `LookAtAndZoomToFit`.
    pub source:    AnimationSource,
    /// The entity this animation framed, or `None` for a raw `PlayAnimation`.
    pub target:    Option<Entity>,
    /// Playback owner that closes this lifecycle episode.
    pub owner:     SequenceOwner,
    /// Direction of the episode being closed.
    pub direction: SequenceDirection,
    /// Raw retained playback timing at completion or interruption.
    pub timing:    CameraEventTiming,
    /// Why the animation stopped: completed naturally, or cancelled.
    pub reason:    AnimationReason,
}

/// Why an [`AnimationEnd`] fired.
#[derive(Clone, Debug, Reflect)]
pub enum AnimationReason {
    /// The retained camera journey ran to completion.
    Completed,
    /// The animation was interrupted before it could complete (either by
    /// external camera input or by a new `PlayAnimation` superseding it).
    Cancelled {
        /// Stable identity of the interrupted stage.
        interrupted_stage_id: SequenceStageId,
        /// The [`CameraMove`] that was in progress when the animation was
        /// cancelled.
        interrupted_move:     CameraMove,
    },
}

/// Emitted when an incoming animation request is rejected.
#[derive(EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct AnimationRejected {
    /// The camera that rejected the animation.
    #[event_target]
    pub camera: Entity,
    /// The source of the rejected request.
    pub source: AnimationSource,
    /// The entity the rejected request would have framed, or `None` for a raw
    /// `PlayAnimation`.
    pub target: Option<Entity>,
    /// Exact reason retained admission did not accept this request.
    pub reason: AnimationRejectionReason,
}

/// `AnimationRejectionReason` identifies the exact failure reported by
/// [`AnimationRejected`].
#[derive(Clone, Debug, Reflect)]
pub enum AnimationRejectionReason {
    /// The request did not contain any authored movement.
    EmptySequence,
    /// A higher-level request could not construct its camera move.
    InvalidMove(CameraMoveError),
    /// The requested entity has neither supported camera controller.
    NoCameraController,
    /// A free-flight camera has no `CameraBasis`.
    MissingCameraBasis,
    /// The requested entity contains both controller kinds.
    ConflictingCameraControllers,
    /// A higher-level request could not prepare its target data.
    RequestPreparationFailed(CameraRequestPreparationError),
    /// Retained endpoint preparation could not produce a camera pose.
    PreparationFailed(CameraEvaluationError),
    /// A native facade request lost to the configured conflict policy.
    NativeConflict(AnimationConflictPolicy),
    /// A selected sequence driver currently owns the camera.
    DriverOwned {
        /// Selected producer that owns the camera sequence.
        driver: Entity,
    },
}

/// `CameraRequestPreparationError` identifies an exact high-level preparation
/// failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum CameraRequestPreparationError {
    /// The requested camera entity no longer has a `Camera` component.
    MissingCamera,
    /// The requested camera has no projection component.
    MissingProjection,
    /// The requested target no longer has usable geometry.
    MissingTargetGeometry,
    /// The requested target no longer has a transform.
    MissingTargetTransform,
    /// The fit solver could not determine an active viewport.
    ViewportUnavailable,
    /// The fit solver found every target point behind the camera.
    PointsBehindCamera,
    /// The requested camera projection cannot solve this fit operation.
    UnsupportedProjection,
}

/// Emitted when an individual `CameraMove` begins.
#[derive(EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct CameraMoveBegin {
    /// The camera being animated.
    #[event_target]
    pub camera:           Entity,
    /// Stable identity of the authored stage that crossed its begin boundary.
    pub stage_id:         SequenceStageId,
    /// Owner that applied the raw traversal.
    pub owner:            SequenceOwner,
    /// Direction that crossed this boundary.
    pub direction:        SequenceDirection,
    /// The `CameraMove` step that is starting.
    pub camera_move:      CameraMove,
    /// Exact authored elapsed time of this stage boundary.
    pub boundary_elapsed: SequenceTime,
    /// Boundary position and complete sequence duration.
    pub timing:           CameraEventTiming,
}

/// Emitted when an individual `CameraMove` completes.
#[derive(EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct CameraMoveEnd {
    /// The camera that finished this move step.
    #[event_target]
    pub camera:           Entity,
    /// Stable identity of the authored stage that crossed its end boundary.
    pub stage_id:         SequenceStageId,
    /// Owner that applied the raw traversal.
    pub owner:            SequenceOwner,
    /// Direction that crossed this boundary.
    pub direction:        SequenceDirection,
    /// The `CameraMove` step that completed.
    pub camera_move:      CameraMove,
    /// Exact authored elapsed time of this stage boundary.
    pub boundary_elapsed: SequenceTime,
    /// Boundary position and complete sequence duration.
    pub timing:           CameraEventTiming,
}
