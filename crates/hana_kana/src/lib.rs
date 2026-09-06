//! # `hana_kana`
//!
//! Ergonomic, opinionated utilities for Bevy — type-safe math and cascade values.
//!
//! `hana_kana` is a growing collection of ergonomic utilities for Bevy projects.
//! Enable features to pull in what you need.
//!
//! ## Features
//!
//! - **`math`** (default) — zero-cost newtype wrappers around Bevy math primitives that prevent
//!   accidental mixing at compile time.
//! - **`tween`** — sequence position movement produced by `bevy_tween`, sequenced before movement
//!   application and domain evaluation.
//!
//! [`Cascade`] and the sequence types are compiled under every feature combination. A [`Cascade`]
//! value either inherits from the next lower-precedence scope or overrides it; [`CascadePlugin`]
//! keeps the matching [`Resolved`] cache across the [`CascadeFrom`] relationship.
//!
//! Disable defaults to pick only what you need:
//!
//! ```toml
//! hana_kana = { version = "0.4.0", default-features = false, features = ["math"] }
//! ```

mod cascade;
mod easing;
#[cfg(feature = "math")]
mod math;
/// Convenience re-exports for glob imports.
pub mod prelude;
mod sequence;
#[cfg(any(feature = "tween", test))]
mod tween;

pub use cascade::CASCADE_DEPTH_LIMIT;
pub use cascade::Cascade;
pub use cascade::CascadeAttribute;
pub use cascade::CascadeChildren;
pub use cascade::CascadeDefault;
pub use cascade::CascadeEntityCommandsExt;
pub use cascade::CascadeFrom;
pub use cascade::CascadePlugin;
pub use cascade::CascadeRootResource;
pub use cascade::CascadeSet;
pub use cascade::Resolved;
pub use cascade::resolve_cascade;
pub use cascade::resolve_cascade_ref;
pub use cascade::resolve_entity_cascade;
pub use cascade::resolved_cascade;
pub use easing::Easing;
pub use easing::EasingCurve;
pub use easing::EasingCurveBuilder;
pub use easing::EasingCurveError;
pub use easing::EasingInput;
pub use easing::EasingInterpolation;
pub use easing::EasingKnot;
pub use easing::EasingMapping;
pub use easing::EasingOutput;
pub use easing::EasingPlugin;
pub use easing::EasingSample;
pub use easing::EasingSampler;
pub use easing::EasingSlope;
pub use easing::EasingSlopes;
#[cfg(feature = "math")]
pub use math::Angle;
#[cfg(feature = "math")]
pub use math::AngleError;
#[cfg(feature = "math")]
pub use math::Displacement;
#[cfg(feature = "math")]
pub use math::Orientation;
#[cfg(feature = "math")]
pub use math::OrientationError;
#[cfg(feature = "math")]
pub use math::Position;
#[cfg(feature = "math")]
pub use math::ScreenPosition;
#[cfg(feature = "math")]
pub use math::ToF32;
#[cfg(feature = "math")]
pub use math::ToF64;
#[cfg(feature = "math")]
pub use math::ToI32;
#[cfg(feature = "math")]
pub use math::ToU8;
#[cfg(feature = "math")]
pub use math::ToU16;
#[cfg(feature = "math")]
pub use math::ToU32;
#[cfg(feature = "math")]
pub use math::ToUsize;
#[cfg(feature = "math")]
pub use math::Velocity;
pub use sequence::DisplacedDriver;
pub use sequence::DriverRestoration;
pub use sequence::RangeCrossing;
pub use sequence::RangeCrossings;
pub use sequence::RangeCrossingsError;
pub use sequence::RangeEdge;
pub use sequence::RangeTransition;
pub use sequence::SequenceCommand;
pub use sequence::SequenceCommandOutcome;
pub use sequence::SequenceCommandRejected;
pub use sequence::SequenceCommandResponse;
pub use sequence::SequenceCommands;
pub use sequence::SequenceDirection;
pub use sequence::SequenceDriver;
pub use sequence::SequenceDriverClaimRejected;
pub use sequence::SequenceDriverReleased;
pub use sequence::SequenceDriverSelected;
pub use sequence::SequenceDriverTakeover;
pub use sequence::SequenceEasing;
pub use sequence::SequenceEasingCurve;
pub use sequence::SequenceEasingError;
pub use sequence::SequenceEasingSample;
pub use sequence::SequenceEasingSampler;
pub use sequence::SequenceEvaluation;
pub use sequence::SequenceMovement;
pub use sequence::SequenceMovementApplication;
pub use sequence::SequenceMovementError;
pub use sequence::SequenceOwner;
pub use sequence::SequenceOwnership;
pub use sequence::SequencePlayback;
pub use sequence::SequencePlaybackError;
pub use sequence::SequencePlaybackPlugin;
pub use sequence::SequencePlaybackSystems;
pub use sequence::SequencePosition;
pub use sequence::SequencePositionError;
pub use sequence::SequenceRange;
pub use sequence::SequenceScope;
pub use sequence::SequenceScopeError;
pub use sequence::SequenceSeekResponse;
pub use sequence::SequenceSourceState;
pub use sequence::SequenceStageId;
pub use sequence::SequenceStageSpan;
pub use sequence::SequenceStages;
pub use sequence::SequenceStagesRevision;
pub use sequence::SequenceTime;
pub use sequence::SequenceTraversal;
pub use sequence::SequenceUpdate;
#[cfg(any(feature = "tween", test))]
pub use tween::SequencePositionInterpolator;
#[cfg(any(feature = "tween", test))]
pub use tween::SequenceTweenAdapterPlugin;
