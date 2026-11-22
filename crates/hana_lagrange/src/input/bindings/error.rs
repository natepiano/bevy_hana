//! Structured binding-validation error produced when a camera's binding descriptor violates an
//! invariant.
//!
//! Types:
//! - [`BindingsError`] — the public error enum surfaced by each camera's binding validation path.

use thiserror::Error;

/// Structured binding validation error.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BindingsError {
    /// A binding entry did not provide source metadata.
    #[error("binding source metadata is missing")]
    MissingSources,
    /// A held action has motion without engagement.
    #[error("{action} is a held binding but has no engagement binding")]
    HeldMotionMissingEngagement {
        /// Semantic action name.
        action: &'static str,
    },
    /// An impulse action was configured with held engagement.
    #[error("{action} is an impulse binding and cannot have an engagement action")]
    ImpulseEngagement {
        /// Semantic action name.
        action: &'static str,
    },
    /// Held motion and engagement metadata did not match.
    #[error("{action} motion and engagement bindings do not share source metadata")]
    HeldSourceMismatch {
        /// Semantic action name.
        action: &'static str,
    },
    /// A held binding requires and blocks the same gate input.
    #[error("{action} requires and blocks the same gate input")]
    ContradictoryGate {
        /// Semantic action name.
        action: &'static str,
    },
    /// A scale modifier or input gain value was invalid.
    #[error("binding scale modifier or input gain value is invalid")]
    InvalidScale,
    /// A dead-zone modifier used unsupported thresholds.
    #[error("binding dead-zone thresholds are invalid")]
    InvalidDeadZone,
}

impl BindingsError {
    /// Returns the semantic action name attached to the error, when available.
    #[must_use]
    pub const fn action_name(&self) -> Option<&'static str> {
        match self {
            Self::HeldMotionMissingEngagement { action }
            | Self::ImpulseEngagement { action }
            | Self::HeldSourceMismatch { action }
            | Self::ContradictoryGate { action } => Some(*action),
            Self::MissingSources | Self::InvalidScale | Self::InvalidDeadZone => None,
        }
    }
}
