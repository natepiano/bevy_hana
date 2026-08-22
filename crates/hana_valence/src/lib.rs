//! Anchor-point relationships for animatable Bevy assemblies.
//!
//! `hana_valence` lets authored geometry expose validated local anchor frames and edges,
//! then connects entities by component data. The contract is
//! [`ResolvedAnchorGeometry`] on each entity, not dynamic dispatch or a global
//! anchor table. Providers fill one `ResolvedAnchorGeometry` per source entity,
//! drivers animate [`AnchorPose`], and consumers run their resolver systems in
//! [`AnchorSystems`].
//!
//! Consumers own anchor-provider, resolver, and transform-propagation wiring.
//! [`ArrangementPlugin`] installs the asset-backed scene support used by
//! [`ArrangementCommandsExt`], while [`FoldPlugin`] composes it with retained
//! fold playback, shared driver arbitration, and fold system ordering:
//!
//! ```rust,ignore
//! use bevy::app::PostUpdate;
//! use bevy::transform::TransformSystems;
//! use hana_valence::AnchorSystems;
//!
//! app.configure_sets(
//!     PostUpdate,
//!     (
//!         AnchorSystems::FillGeometry,
//!         AnchorSystems::AnimatePose,
//!         AnchorSystems::Resolve,
//!     )
//!         .chain()
//!         .before(TransformSystems::Propagate),
//! );
//! ```
//!
//! Animator systems may tween two valence inputs: [`AnchorPose`] on entities
//! that carry no [`Hinge`], and [`bevy_transform::prelude::Transform`] on
//! entities that do not carry [`AnchoredTo`]. Register those systems in
//! [`AnchorSystems::AnimatePose`] so their writes happen before
//! [`resolve_anchors`] runs in [`AnchorSystems::Resolve`].
//!
//! A [`Hinge`] is not animated directly. It stores the two endpoints a member
//! travels between — its base pose and its folded pose — and
//! [`hinge_to_pose`] derives the current [`AnchorPose`] from the
//! [`EasedFoldFraction`] that [`FoldSequencePlayback`] resolved for that member
//! in `Update`, or from the base endpoint when no sequence stages it. A fold
//! recipe such as [`Accordion`], [`Coil`], or [`Wrap`] replaces the folded
//! endpoint of every selected member at once through
//! [`ArrangementCommandsExt::apply_fold_recipe`].
//!
//! Because the angle is derived in `PostUpdate` from the live [`Hinge`],
//! replacing that component before [`hinge_to_pose`] changes the pose in the
//! same frame even at an unchanged sequence position. An unchanged pose is not
//! rewritten, so an idle fold performs no write; a pose that does change
//! discards a direct `AnchorPose` tween on that hinged entity, and debug builds
//! warn when it replaces an earlier same-frame `AnchorPose` write. Entities
//! without a [`Hinge`] are never written, so a hand-driven `AnchorPose` on an
//! unhinged entity is left alone. `bevy_animation` property adapters can be
//! added later without changing this component contract.
//!
//! An [`Arrangement`] is a non-spatial controller. A [`Member`] is stored on
//! each member root and points to that controller; Bevy maintains the reverse
//! [`Members`] collection on the controller. Membership is independent from
//! the physical [`AnchoredTo`] relationship, so membership order never implies
//! a physical attachment target.
//!
//! Anchor naming has three tiers. Generated geometry should use sites derived
//! from adjacency and never require authored names. Hand-authored regular
//! geometry should use provider names such as `Anchor::TopLeft` when offered.
//! One-off geometry can use raw [`AnchorSite`] values. Pick the highest tier that
//! matches the data you own; reusable recipes ask [`ResolvedAnchorGeometry`]
//! which edge is shared with the predecessor instead of hardcoding ids.
//!
//! Resolver math for an entity with [`AnchoredTo`] is:
//!
//! ```text
//! target_world = parent.global * parent.geometry.frame(target_site).position
//! source_local = child.geometry.frame(source_site).position
//! base         = parent.global.rotation * target_frame.orientation
//! rot          = base * pose.rotation * source_frame.orientation.inverse()
//! offset_eff   = resolved_anchor_offset or anchored_to.offset
//! child.translation = target_world + base * (offset_eff + pose.translation)
//!                   - rot * (child_global_scale * source_local)
//! child.rotation    = rot
//! ```
//!
//! `child_global_scale` supports uniform scale on the child entity. Non-uniform
//! child scale is unsupported because the source-anchor subtraction applies
//! scale in the child frame before rotating into the target-anchor frame.
//!
//! `offset_eff` and `pose.translation` are evaluated in the target-anchor frame
//! and are independent of `pose.rotation`. For example, with identity frames,
//! a target anchor at `(3, 3, 0)`, and raw offset `(0.25, -0.5, 0)`, the child
//! anchor lands at `(3.25, 2.5, 0)`. Unit conversion, DPI conversion, and
//! coordinate-system sign changes happen in provider crates before data reaches
//! this resolver.
//!
//! [`resolve_anchors`] writes local [`bevy_transform::prelude::Transform`]
//! values. Same-frame reads of anchored entities'
//! [`bevy_transform::prelude::GlobalTransform`] components are stale until the
//! consumer runs transform propagation after [`AnchorSystems::Resolve`].
//!
//! [`ResolvedAnchorWorld`] is recomputed every frame, never
//! change-detection-gated. Entities resolved in the current frame get cache
//! points from their just-resolved global transform. Entities that carry the
//! cache but were not resolved in the current frame get points from the previous
//! propagation pass's `GlobalTransform`, the same one-frame staleness that
//! applies to every `GlobalTransform` read by the resolver because
//! [`AnchorSystems::Resolve`] runs before `TransformSystems::Propagate`. The
//! cache has the same freshness as the resolve pass that writes it.

// Lets the shared `../fixtures.rs` include reference this crate by name from the
// `resolve` unit tests, matching the external examples and integration test.
extern crate self as hana_valence;

mod arrangement;
mod attachment;
mod fold;
mod geometry;
mod hinge;
mod pose;
mod providers;
mod relation;
mod resolve;

pub use arrangement::Arrangement;
pub use arrangement::ArrangementCommandsExt;
pub use arrangement::ArrangementConnection;
pub use arrangement::ArrangementConnectionDisplacement;
pub use arrangement::ArrangementError;
pub use arrangement::ArrangementMemberEntities;
pub use arrangement::ArrangementPlan;
pub use arrangement::ArrangementPlugin;
pub use arrangement::ArrangementProvider;
pub use arrangement::HingeClearance;
pub use arrangement::Member;
pub use arrangement::MemberBinding;
pub use arrangement::Members;
pub use arrangement::PlannedFoldSequence;
pub use arrangement::Provides;
pub use arrangement::RetainedProviderKnowledge;
pub use attachment::AttachmentResolveAction;
pub use attachment::AttachmentResolveCandidate;
pub use attachment::AttachmentResolveDiagnostic;
pub use attachment::AttachmentResolveDiagnostics;
pub use attachment::AttachmentResolveReasons;
pub use attachment::AttachmentResolverScratch;
pub use attachment::resolve_attachments;
pub use attachment::resolve_attachments_with_scratch;
pub use bevy_kana::Angle;
pub use bevy_kana::Displacement;
pub use bevy_kana::Easing;
pub use bevy_kana::EasingCurve;
pub use bevy_kana::EasingCurveBuilder;
pub use bevy_kana::EasingCurveError;
pub use bevy_kana::EasingInput;
pub use bevy_kana::EasingInterpolation;
pub use bevy_kana::EasingKnot;
pub use bevy_kana::EasingOutput;
pub use bevy_kana::EasingSample;
pub use bevy_kana::EasingSlope;
pub use bevy_kana::EasingSlopes;
pub use bevy_kana::Orientation;
pub use bevy_kana::Position;
pub use bevy_kana::RangeCrossing;
pub use bevy_kana::RangeCrossings;
pub use bevy_kana::RangeEdge;
pub use bevy_kana::SequenceCommand;
pub use bevy_kana::SequenceCommandOutcome;
pub use bevy_kana::SequenceCommandRejected;
pub use bevy_kana::SequenceCommandResponse;
pub use bevy_kana::SequenceDirection;
pub use bevy_kana::SequenceDriver;
pub use bevy_kana::SequenceDriverTakeover;
pub use bevy_kana::SequenceEasing;
pub use bevy_kana::SequenceEasingError;
pub use bevy_kana::SequenceEasingSample;
pub use bevy_kana::SequenceEvaluation;
pub use bevy_kana::SequenceMovement;
pub use bevy_kana::SequenceOwner;
pub use bevy_kana::SequenceOwnership;
pub use bevy_kana::SequencePosition;
pub use bevy_kana::SequenceScope;
pub use bevy_kana::SequenceSourceState;
pub use bevy_kana::SequenceStageId;
pub use bevy_kana::SequenceStages;
pub use bevy_kana::SequenceTime;
pub use fold::Accordion;
pub use fold::Coil;
pub use fold::EasedFoldFraction;
pub use fold::FoldAssignment;
pub use fold::FoldAuthorError;
pub use fold::FoldBoundary;
pub use fold::FoldBoundaryRecord;
pub use fold::FoldCommands;
pub use fold::FoldEndpoint;
pub use fold::FoldEndpointReached;
pub use fold::FoldEvaluationError;
pub use fold::FoldEventTiming;
pub use fold::FoldFractionScratch;
pub use fold::FoldGroup;
pub use fold::FoldGroups;
pub use fold::FoldLedger;
pub use fold::FoldMemberBegin;
pub use fold::FoldMemberBoundary;
pub use fold::FoldMemberEnd;
pub use fold::FoldMemberFraction;
pub use fold::FoldMemberSample;
pub use fold::FoldMemberTrack;
pub use fold::FoldPlugin;
pub use fold::FoldRecipe;
pub use fold::FoldRecipeCapability;
pub use fold::FoldSegment;
pub use fold::FoldSegmentProgress;
pub use fold::FoldSequence;
pub use fold::FoldSequenceBuilder;
pub use fold::FoldSequencePlayback;
pub use fold::FoldStage;
pub use fold::FoldStageBegin;
pub use fold::FoldStageEnd;
pub use fold::FoldSystems;
pub use fold::FoldTarget;
pub use fold::FoldTiming;
pub use fold::NoCapability;
pub use fold::ProviderCapability;
pub use fold::WindingClearance;
pub use fold::Wrap;
pub use fold::evaluate_fold_angle;
pub use fold::fold_fraction;
pub use geometry::AnchorFrame;
pub use geometry::AnchorSite;
pub use geometry::Edge;
pub use geometry::GeometryError;
pub use geometry::ResolvedAnchorGeometry;
pub use hinge::Hinge;
pub use hinge::HingeError;
pub use hinge::HingePoseReported;
pub use hinge::hinge_to_pose;
pub use pose::AnchorPose;
pub use pose::AnchorSystems;
pub use pose::ResolvedAnchorWorld;
pub use providers::QuadCell;
pub use providers::QuadFoldGroupSelection;
pub use providers::QuadSheet;
pub use providers::TriangleCell;
pub use providers::TriangleCellOrientation;
pub use providers::TriangleFoldGroupSelection;
pub use providers::TriangleSheet;
pub use relation::AnchoredHere;
pub use relation::AnchoredTo;
pub use relation::ResolvedAnchorOffset;
pub use resolve::AnchorResolveDiagnostics;
pub use resolve::AnchorResolveSkip;
pub use resolve::AnchorResolverScratch;
pub use resolve::resolve_anchors;
