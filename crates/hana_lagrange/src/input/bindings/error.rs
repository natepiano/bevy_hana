//! Structured binding-validation error produced when a camera's binding descriptor violates an
//! invariant.
//!
//! Types:
//! - [`BindingsError`] — the public error enum surfaced by each camera's binding validation path.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

/// Structured binding validation error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingsError {
    /// A binding entry did not provide source metadata.
    MissingSources,
    /// A held action has motion without engagement.
    HeldMotionMissingEngagement {
        /// Semantic action name.
        action: &'static str,
    },
    /// An impulse action was configured with held engagement.
    ImpulseEngagement {
        /// Semantic action name.
        action: &'static str,
    },
    /// Held motion and engagement metadata did not match.
    HeldSourceMismatch {
        /// Semantic action name.
        action: &'static str,
    },
    /// A held binding requires and blocks the same gate input.
    ContradictoryGate {
        /// Semantic action name.
        action: &'static str,
    },
    /// A scale modifier or input gain value was invalid.
    InvalidScale,
    /// A dead-zone modifier used unsupported thresholds.
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

impl Display for BindingsError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSources => formatter.write_str("binding source metadata is missing"),
            Self::HeldMotionMissingEngagement { action } => {
                write!(
                    formatter,
                    "{action} is a held binding but has no engagement binding"
                )
            },
            Self::ImpulseEngagement { action } => {
                write!(
                    formatter,
                    "{action} is an impulse binding and cannot have an engagement action"
                )
            },
            Self::HeldSourceMismatch { action } => {
                write!(
                    formatter,
                    "{action} motion and engagement bindings do not share source metadata"
                )
            },
            Self::ContradictoryGate { action } => {
                write!(
                    formatter,
                    "{action} requires and blocks the same gate input"
                )
            },
            Self::InvalidScale => {
                formatter.write_str("binding scale modifier or input gain value is invalid")
            },
            Self::InvalidDeadZone => {
                formatter.write_str("binding dead-zone thresholds are invalid")
            },
        }
    }
}

impl Error for BindingsError {}
