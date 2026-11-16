//! # `hana_kana`
//!
//! Ergonomic, opinionated utilities for Bevy — type-safe math and cascade values.
//!
//! `hana_kana` is a growing collection of ergonomic utilities for Bevy projects.
//! Enable features to pull in what you need.
//!
//! ## Features
//!
//! - **`bevy`** (default) — cascade, easing and sequence support.
//! - **`math`** (default) — zero-cost newtype wrappers around Bevy math primitives that prevent
//!   accidental mixing at compile time.
//! - **`tween`** — sequence position movement produced by `bevy_tween`, sequenced before movement
//!   application and domain evaluation.
//!
//! The default `bevy` feature provides cascade, easing and sequence types. Cast traits
//! remain available without default features. A `Cascade`
//! value either inherits from the next lower-precedence scope or overrides it; `CascadePlugin`
//! keeps the matching `Resolved` cache across the `CascadeFrom` relationship.
//!
//! Disable defaults to pick only what you need:
//!
//! ```toml
//! hana_kana = { version = "0.4.0", default-features = false, features = ["math"] }
//! ```

#[cfg(feature = "bevy")]
mod cascade;
#[cfg(feature = "bevy")]
mod easing;
mod math;
/// Convenience re-exports for glob imports.
#[cfg(feature = "bevy")]
pub mod prelude;
mod readout;
#[cfg(feature = "bevy")]
mod sequence;
#[cfg(all(feature = "bevy", any(feature = "tween", test)))]
mod tween;

#[cfg(feature = "bevy")]
pub use cascade::CASCADE_DEPTH_LIMIT;
#[cfg(feature = "bevy")]
pub use cascade::Cascade;
#[cfg(feature = "bevy")]
pub use cascade::CascadeAttribute;
#[cfg(feature = "bevy")]
pub use cascade::CascadeChildren;
#[cfg(feature = "bevy")]
pub use cascade::CascadeDefault;
#[cfg(feature = "bevy")]
pub use cascade::CascadeEntityCommandsExt;
#[cfg(feature = "bevy")]
pub use cascade::CascadeFrom;
#[cfg(feature = "bevy")]
pub use cascade::CascadePlugin;
#[cfg(feature = "bevy")]
pub use cascade::CascadeRootResource;
#[cfg(feature = "bevy")]
pub use cascade::CascadeSet;
#[cfg(feature = "bevy")]
pub use cascade::Resolved;
#[cfg(feature = "bevy")]
pub use cascade::resolve_cascade;
#[cfg(feature = "bevy")]
pub use cascade::resolve_cascade_ref;
#[cfg(feature = "bevy")]
pub use cascade::resolve_entity_cascade;
#[cfg(feature = "bevy")]
pub use cascade::resolve_entity_cascade_with;
#[cfg(feature = "bevy")]
pub use cascade::resolved_cascade;
#[cfg(feature = "bevy")]
pub use easing::Easing;
#[cfg(feature = "bevy")]
pub use easing::EasingCurve;
#[cfg(feature = "bevy")]
pub use easing::EasingCurveBuilder;
#[cfg(feature = "bevy")]
pub use easing::EasingCurveError;
#[cfg(feature = "bevy")]
pub use easing::EasingInput;
#[cfg(feature = "bevy")]
pub use easing::EasingInterpolation;
#[cfg(feature = "bevy")]
pub use easing::EasingKnot;
#[cfg(feature = "bevy")]
pub use easing::EasingMapping;
#[cfg(feature = "bevy")]
pub use easing::EasingOutput;
#[cfg(feature = "bevy")]
pub use easing::EasingPlugin;
#[cfg(feature = "bevy")]
pub use easing::EasingSample;
#[cfg(feature = "bevy")]
pub use easing::EasingSampler;
#[cfg(feature = "bevy")]
pub use easing::EasingSlope;
#[cfg(feature = "bevy")]
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
pub use math::ToF32;
pub use math::ToF64;
pub use math::ToI32;
pub use math::ToI64;
pub use math::ToU8;
pub use math::ToU16;
pub use math::ToU32;
pub use math::ToUsize;
#[cfg(feature = "math")]
pub use math::Velocity;
pub use readout::ThreeSignificantDigits;
#[cfg(feature = "bevy")]
pub use sequence::DisplacedDriver;
#[cfg(feature = "bevy")]
pub use sequence::DriverRestoration;
#[cfg(feature = "bevy")]
pub use sequence::RangeCrossing;
#[cfg(feature = "bevy")]
pub use sequence::RangeCrossings;
#[cfg(feature = "bevy")]
pub use sequence::RangeCrossingsError;
#[cfg(feature = "bevy")]
pub use sequence::RangeEdge;
#[cfg(feature = "bevy")]
pub use sequence::RangeTransition;
#[cfg(feature = "bevy")]
pub use sequence::SequenceCommand;
#[cfg(feature = "bevy")]
pub use sequence::SequenceCommandOutcome;
#[cfg(feature = "bevy")]
pub use sequence::SequenceCommandRejected;
#[cfg(feature = "bevy")]
pub use sequence::SequenceCommandResponse;
#[cfg(feature = "bevy")]
pub use sequence::SequenceCommands;
#[cfg(feature = "bevy")]
pub use sequence::SequenceDirection;
#[cfg(feature = "bevy")]
pub use sequence::SequenceDriver;
#[cfg(feature = "bevy")]
pub use sequence::SequenceDriverClaimRejected;
#[cfg(feature = "bevy")]
pub use sequence::SequenceDriverReleased;
#[cfg(feature = "bevy")]
pub use sequence::SequenceDriverSelected;
#[cfg(feature = "bevy")]
pub use sequence::SequenceDriverTakeover;
#[cfg(feature = "bevy")]
pub use sequence::SequenceEasing;
#[cfg(feature = "bevy")]
pub use sequence::SequenceEasingCurve;
#[cfg(feature = "bevy")]
pub use sequence::SequenceEasingError;
#[cfg(feature = "bevy")]
pub use sequence::SequenceEasingSample;
#[cfg(feature = "bevy")]
pub use sequence::SequenceEasingSampler;
#[cfg(feature = "bevy")]
pub use sequence::SequenceEvaluation;
#[cfg(feature = "bevy")]
pub use sequence::SequenceMovement;
#[cfg(feature = "bevy")]
pub use sequence::SequenceMovementApplication;
#[cfg(feature = "bevy")]
pub use sequence::SequenceMovementError;
#[cfg(feature = "bevy")]
pub use sequence::SequenceOwner;
#[cfg(feature = "bevy")]
pub use sequence::SequenceOwnership;
#[cfg(feature = "bevy")]
pub use sequence::SequencePlayback;
#[cfg(feature = "bevy")]
pub use sequence::SequencePlaybackError;
#[cfg(feature = "bevy")]
pub use sequence::SequencePlaybackPlugin;
#[cfg(feature = "bevy")]
pub use sequence::SequencePlaybackSystems;
#[cfg(feature = "bevy")]
pub use sequence::SequencePosition;
#[cfg(feature = "bevy")]
pub use sequence::SequencePositionError;
#[cfg(feature = "bevy")]
pub use sequence::SequenceRange;
#[cfg(feature = "bevy")]
pub use sequence::SequenceScope;
#[cfg(feature = "bevy")]
pub use sequence::SequenceScopeError;
#[cfg(feature = "bevy")]
pub use sequence::SequenceSeekResponse;
#[cfg(feature = "bevy")]
pub use sequence::SequenceSourceState;
#[cfg(feature = "bevy")]
pub use sequence::SequenceStageId;
#[cfg(feature = "bevy")]
pub use sequence::SequenceStageSpan;
#[cfg(feature = "bevy")]
pub use sequence::SequenceStages;
#[cfg(feature = "bevy")]
pub use sequence::SequenceStagesRevision;
#[cfg(feature = "bevy")]
pub use sequence::SequenceTime;
#[cfg(feature = "bevy")]
pub use sequence::SequenceTraversal;
#[cfg(feature = "bevy")]
pub use sequence::SequenceUpdate;
#[cfg(all(feature = "bevy", any(feature = "tween", test)))]
pub use tween::SequencePositionInterpolator;
#[cfg(all(feature = "bevy", any(feature = "tween", test)))]
pub use tween::SequenceTweenAdapterPlugin;
